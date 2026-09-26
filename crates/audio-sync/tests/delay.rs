//! Tests of the delay probe on synthetic pairs. The signals are made in the
//! test, so no recording is necessary.

use audio_sync::{SyncProbe, SyncProbeConfig, SyncProbeState};

const RATE: usize = 16_000;
const MAX_LAG_SAMPLES: usize = RATE * 400 / 1000;
const STEP_SAMPLES: isize = (RATE / 100) as isize;
const CHUNK: usize = RATE / 10;

fn excitation(len: usize) -> Vec<f32> {
    let mut state = 0x1234_5678u32;
    (0..len)
        .map(|idx| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let noise = (state as f32 / u32::MAX as f32) * 2.0 - 1.0;
            let pulse = if idx % 257 == 0 { 0.75 } else { 0.0 };
            0.3 * noise + pulse
        })
        .collect()
}

/// A delayed, scaled copy of `reference`, with the delay in samples that
/// `delay_at` gives for each output index.
fn observed(reference: &[f32], delay_at: impl Fn(usize) -> usize) -> Vec<f32> {
    (0..reference.len())
        .map(|idx| {
            idx.checked_sub(delay_at(idx))
                .map_or(0.0, |source| 0.4 * reference[source])
        })
        .collect()
}

fn probe() -> SyncProbe {
    let config = SyncProbeConfig::for_max_lag(RATE as u32, MAX_LAG_SAMPLES);
    assert_eq!(config.max_lag_samples, MAX_LAG_SAMPLES);
    assert!(config.window_samples > MAX_LAG_SAMPLES);
    SyncProbe::new(config)
}

/// Feeds the pair in chunks of 100 ms and returns the stable lag after each
/// chunk, with the sample index at the chunk end.
fn stable_lags(reference: &[f32], observed: &[f32]) -> Vec<(usize, Option<isize>)> {
    let mut probe = probe();
    let mut lags = Vec::new();
    let mut end = 0;
    let mut stable = None;
    for (reference, observed) in reference.chunks(CHUNK).zip(observed.chunks(CHUNK)) {
        end += reference.len();
        if let Some(event) = probe.observe(reference, observed) {
            let snapshot = event.snapshot();
            stable = match snapshot.state {
                SyncProbeState::Locked | SyncProbeState::Holdover => snapshot.stable_lag_samples,
                _ => None,
            };
        }
        lags.push((end, stable));
    }
    lags
}

fn within_one_step(lag: Option<isize>, expected: usize) -> bool {
    lag.is_some_and(|lag| (lag - expected as isize).abs() <= STEP_SAMPLES)
}

#[test]
fn probe_finds_a_250_ms_delay() {
    let delay = RATE * 250 / 1000;
    let reference = excitation(RATE * 10);
    let observed = observed(&reference, |_| delay);

    let lags = stable_lags(&reference, &observed);
    let (_, last) = *lags.last().unwrap();
    assert!(
        within_one_step(last, delay),
        "stable lag {last:?}, expected {delay}"
    );
}

#[test]
fn probe_follows_a_delay_that_changes_from_60_ms_to_250_ms() {
    let first = RATE * 60 / 1000;
    let second = RATE * 250 / 1000;
    let change = RATE * 20;
    let reference = excitation(RATE * 40);
    let observed = observed(&reference, |idx| if idx < change { first } else { second });

    let lags = stable_lags(&reference, &observed);
    let before = lags.iter().rfind(|(end, _)| *end <= change).unwrap().1;
    assert!(
        within_one_step(before, first),
        "stable lag {before:?} before the change"
    );
    let (_, last) = *lags.last().unwrap();
    assert!(
        within_one_step(last, second),
        "stable lag {last:?} after the change"
    );
}
