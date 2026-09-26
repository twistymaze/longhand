//! The joiner under simulated device clocks (KTD5 of the system audio plan).
//!
//! Each speaker sample carries a tag: its index in the speaker stream plus
//! one, so 0 is the silence that the joiner pads with. The simulation knows
//! when each sample was made on the microphone clock, so the channel offset
//! of an emitted speaker sample is its output position minus that time. With
//! the two clocks equal and no stall, the offset is 0.

use audio::joiner::{Joiner, MAX_SPEAKER_QUEUE_MS};

const RATE: u32 = 16_000;
/// 20 ms at 16 kHz, the frame size of both streams.
const FRAME: usize = 320;

/// A stall of the speaker stream: no packet arrives from `from_ms` for
/// `for_ms`, and then the late packets all arrive at once.
#[derive(Clone, Copy)]
struct Stall {
    from_ms: u64,
    for_ms: u64,
}

#[derive(Debug, Default)]
struct Report {
    mic_in: u64,
    mic_out: u64,
    /// Largest and smallest channel offset of an emitted speaker sample, in
    /// microphone samples.
    max_offset: f64,
    min_offset: f64,
    dropped: u64,
    padded: u64,
    speaker_samples_out: u64,
}

/// Run `seconds` of paired capture. The speaker clock runs `drift_ppm`
/// faster than the microphone clock (slower when negative).
fn simulate(seconds: u64, drift_ppm: f64, stalls: &[Stall]) -> Report {
    let speed = 1.0 + drift_ppm / 1e6;
    let mut joiner: Joiner<u64> = Joiner::new(RATE);
    let mut report = Report {
        max_offset: f64::MIN,
        min_offset: f64::MAX,
        ..Report::default()
    };
    let mic_frames = seconds * u64::from(RATE) / FRAME as u64;
    let mut packet: u64 = 0;
    let mut right = Vec::with_capacity(FRAME);
    let mut buffer = Vec::with_capacity(FRAME);
    for m in 0..mic_frames {
        // The microphone frame m arrives at this time, in microphone samples.
        let now = (m + 1) as f64 * FRAME as f64;
        // Deliver each speaker packet that has arrived by now.
        loop {
            let made = (packet + 1) as f64 * FRAME as f64 / speed;
            let arrives = stalls
                .iter()
                .find(|s| {
                    let from = (s.from_ms * u64::from(RATE) / 1000) as f64;
                    let to = ((s.from_ms + s.for_ms) * u64::from(RATE) / 1000) as f64;
                    made >= from && made < to
                })
                .map_or(made, |s| {
                    ((s.from_ms + s.for_ms) * u64::from(RATE) / 1000) as f64
                });
            if arrives > now {
                break;
            }
            buffer.clear();
            let first = packet * FRAME as u64;
            buffer.extend((first..first + FRAME as u64).map(|n| n + 1));
            report.dropped += joiner.push_speaker(&buffer) as u64;
            packet += 1;
        }
        report.mic_in += FRAME as u64;
        report.padded += joiner.pair(FRAME, &mut right) as u64;
        assert_eq!(right.len(), FRAME, "one speaker sample for each mic sample");
        let position = m * FRAME as u64;
        // The speaker samples of one frame come first and hold consecutive
        // tags, so the offset changes monotonically inside the frame: its
        // extremes are at the first and the last speaker sample.
        let speaker = right.iter().take_while(|&&tag| tag != 0).count();
        assert!(
            right[speaker..].iter().all(|&tag| tag == 0),
            "padding ends a frame"
        );
        if speaker > 0 {
            assert_eq!(right[speaker - 1] - right[0], speaker as u64 - 1);
            for i in [0, speaker - 1] {
                let made = (right[i] - 1) as f64 / speed;
                let offset = (position + i as u64) as f64 - made;
                report.max_offset = report.max_offset.max(offset);
                report.min_offset = report.min_offset.min(offset);
            }
        }
        report.speaker_samples_out += speaker as u64;
        report.mic_out += right.len() as u64;
    }
    report
}

fn bound() -> f64 {
    f64::from(RATE * MAX_SPEAKER_QUEUE_MS / 1000)
}

#[test]
fn equal_clocks_give_no_offset_and_no_drop() {
    let report = simulate(60, 0.0, &[]);
    assert_eq!(report.mic_out, report.mic_in);
    assert_eq!((report.dropped, report.padded), (0, 0));
    assert_eq!((report.min_offset, report.max_offset), (0.0, 0.0));
}

#[test]
fn three_speaker_stalls_of_60_ms_keep_the_offset_within_60_ms() {
    let stalls = [
        Stall {
            from_ms: 10_000,
            for_ms: 60,
        },
        Stall {
            from_ms: 20_000,
            for_ms: 60,
        },
        Stall {
            from_ms: 30_000,
            for_ms: 60,
        },
    ];
    let report = simulate(40, 0.0, &stalls);
    assert_eq!(report.mic_out, report.mic_in, "no mic frame is dropped");
    assert!(report.padded > 0, "the joiner padded during the stalls");
    assert!(
        report.max_offset <= bound(),
        "offset {} samples is past {} samples",
        report.max_offset,
        bound()
    );
    assert!(report.min_offset >= -bound(), "{report:?}");
}

/// Four hours of frames at 50 ppm in each direction (KTD5 soak). The
/// offset stays within 60 ms for the whole run, and the microphone count
/// equals its input count.
#[test]
fn four_hours_of_drift_in_each_direction_keep_the_offset_within_60_ms() {
    for drift in [50.0, -50.0] {
        let report = simulate(4 * 3_600, drift, &[]);
        assert_eq!(report.mic_out, report.mic_in, "{drift} ppm: {report:?}");
        assert!(
            report.max_offset <= bound() && report.min_offset >= -bound(),
            "{drift} ppm: {report:?}"
        );
        let hours_of_drift = 4.0 * 3_600.0 * f64::from(RATE) * 50e-6;
        if drift > 0.0 {
            // The fast speaker clock makes about 11,520 extra samples; the
            // joiner drops what the 60 ms queue cannot hold.
            assert!(report.dropped > 0, "{report:?}");
            assert!(
                (report.dropped as f64) < hours_of_drift + bound(),
                "{report:?}"
            );
        } else {
            assert_eq!(report.dropped, 0, "{report:?}");
            assert!(report.padded > 0, "{report:?}");
            assert!(
                (report.padded as f64) < hours_of_drift + 2.0 * FRAME as f64,
                "{report:?}"
            );
        }
    }
}
