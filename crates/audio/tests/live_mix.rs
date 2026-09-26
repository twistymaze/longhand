//! The live mix of a stereo segment (KTD11 of the system audio plan): the
//! gain of each channel and the echo gate, on synthetic signals at the
//! microphone's native rate.

use audio::live_mix::{LiveMix, MAX_GAIN, MIN_GAIN};

const RATE: u32 = 48_000;
/// One capture frame: 10 ms.
const FRAME: usize = RATE as usize / 100;

/// A broadband signal with pulses, near 0.26 RMS (anarlog's test signal).
fn signal(len: usize, seed: u32) -> Vec<f32> {
    let mut state = seed;
    (0..len)
        .map(|idx| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let noise = (state as f32 / u32::MAX as f32) * 2.0 - 1.0;
            let pulse = if idx % 257 == 0 { 0.8 } else { 0.0 };
            0.45 * noise + pulse
        })
        .collect()
}

fn scaled(input: &[f32], gain: f32) -> Vec<f32> {
    input.iter().map(|s| s * gain).collect()
}

/// `input` late by `delay` samples and scaled by `gain`, as a speaker echo.
fn delayed(input: &[f32], delay: usize, gain: f32) -> Vec<f32> {
    let mut out = vec![0.0; input.len()];
    for i in delay..input.len() {
        out[i] = gain * input[i - delay];
    }
    out
}

fn add(a: &[f32], b: &[f32]) -> Vec<f32> {
    a.iter().zip(b).map(|(a, b)| a + b).collect()
}

/// One mixed frame and the state of the mix after it.
struct Mixed {
    out: Vec<f32>,
    gated: bool,
    gains: (f32, f32),
}

/// Give `mic` and `speaker` to a new live mix in 10 ms frames.
fn run(mic: &[f32], speaker: &[f32]) -> Vec<Mixed> {
    let mut mix = LiveMix::new(RATE);
    mic.chunks(FRAME)
        .zip(speaker.chunks(FRAME))
        .map(|(m, s)| {
            let out = mix.mix(m, s);
            assert_eq!(out.len(), m.len());
            Mixed {
                out,
                gated: mix.gated(),
                gains: mix.gains(),
            }
        })
        .collect()
}

fn ms_to_frames(ms: usize) -> usize {
    ms / 10
}

/// The echo gate zeroes the microphone once it has heard enough of the
/// system channel for the delay, so the mix holds only the system channel.
fn gate_zeroes_an_echo_late_by(delay_ms: usize) {
    let len = RATE as usize * 3;
    let speaker = signal(len, 0x1234_5678);
    let mic = delayed(&speaker, delay_ms * RATE as usize / 1000, 0.35);
    let frames = run(&mic, &speaker);
    // After the delay and one gate window (120 ms), with a margin.
    let from = ms_to_frames(delay_ms + 200);
    for (n, f) in frames.iter().enumerate().skip(from) {
        assert!(
            f.gated,
            "frame {n} is not gated at a delay of {delay_ms} ms"
        );
        let (_, system_gain) = f.gains;
        let at = n * FRAME;
        for (i, (o, s)) in f.out.iter().zip(&speaker[at..]).enumerate() {
            assert!(
                (o - (system_gain * s).clamp(-1.0, 1.0)).abs() < 1e-6,
                "frame {n} sample {i}: the mix holds microphone audio"
            );
        }
    }
}

#[test]
fn the_gate_zeroes_an_echo_60_ms_late() {
    gate_zeroes_an_echo_late_by(60);
}

#[test]
fn the_gate_zeroes_an_echo_250_ms_late() {
    // Bluetooth output adds 150 to 250 ms.
    gate_zeroes_an_echo_late_by(250);
}

#[test]
fn the_gate_keeps_the_microphone_when_other_speech_dominates() {
    let len = RATE as usize * 3;
    let speaker = signal(len, 0x1234_5678);
    let local = signal(len, 0x8765_4321);
    let mic = add(
        &delayed(&speaker, 60 * RATE as usize / 1000, 0.15),
        &scaled(&local, 0.45),
    );
    let frames = run(&mic, &speaker);
    let gated = frames.iter().filter(|f| f.gated).count();
    assert_eq!(gated, 0, "{gated} of {} frames gated", frames.len());
}

#[test]
fn the_gate_keeps_the_microphone_when_the_system_channel_is_quiet() {
    let len = RATE as usize * 2;
    let speaker = vec![0.001; len];
    let mic = signal(len, 0x1234_5678);
    let frames = run(&mic, &speaker);
    assert!(frames.iter().all(|f| !f.gated));
}

#[test]
fn with_a_silent_system_channel_the_live_mix_is_the_microphone() {
    let len = RATE as usize * 4;
    let mic = scaled(&signal(len, 0x1234_5678), 0.2);
    let speaker = vec![0.0; len];
    let frames = run(&mic, &speaker);
    let out: Vec<f32> = frames.iter().flat_map(|f| f.out.iter().copied()).collect();
    assert_eq!(out, mic);
    assert!(frames.iter().all(|f| f.gains == (1.0, 1.0)));
}

#[test]
fn the_gain_rises_for_a_quiet_system_channel_and_stays_inside_its_bounds() {
    // A loud room and a call just above the speech floor: the balance wants
    // a microphone gain under the lower bound and a system gain near 15.
    let len = RATE as usize * 20;
    let mic = scaled(&signal(len, 0x1234_5678), 1.5);
    let speaker = scaled(&signal(len, 0x0bad_cafe), 0.0135);
    let frames = run(&mic, &speaker);
    for (n, f) in frames.iter().enumerate() {
        let (mic_gain, system_gain) = f.gains;
        assert!(
            (MIN_GAIN..=MAX_GAIN).contains(&mic_gain),
            "frame {n}: microphone gain {mic_gain}"
        );
        assert!(
            (MIN_GAIN..=MAX_GAIN).contains(&system_gain),
            "frame {n}: system gain {system_gain}"
        );
        assert!(f.out.iter().all(|s| (-1.0..=1.0).contains(s)));
    }
    let (mic_gain, system_gain) = frames.last().unwrap().gains;
    assert!(system_gain > 8.0, "system gain {system_gain}");
    assert!(
        (mic_gain - MIN_GAIN).abs() < 0.01,
        "microphone gain {mic_gain}"
    );
    // The gain changes slowly: it does not jump when the levels are known.
    for pair in frames.windows(2) {
        let (a, b) = (pair[0].gains.1, pair[1].gains.1);
        assert!((b - a).abs() < 1.0, "the system gain jumps from {a} to {b}");
    }
}
