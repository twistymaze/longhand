//! Tests of the echo canceller on synthetic signals. The signals are made in
//! the test, so no recording is necessary.

use aec::{BLOCK_SHIFT, EchoCanceller, SAMPLE_RATE};

const RATE: usize = SAMPLE_RATE as usize;

/// A synthetic voice: a harmonic tone with a moving pitch, cut into syllables
/// of about 4 Hz, with a small noise part.
fn voice(len: usize, base_pitch_hz: f32, seed: u32) -> Vec<f32> {
    let mut state = seed.max(1);
    let mut phase = 0.0f32;
    let rate = RATE as f32;
    (0..len)
        .map(|idx| {
            let t = idx as f32 / rate;
            let pitch = base_pitch_hz * (1.0 + 0.08 * (2.0 * std::f32::consts::PI * 0.7 * t).sin());
            phase += 2.0 * std::f32::consts::PI * pitch / rate;
            let mut tone = 0.0;
            for harmonic in 1..=12 {
                let weight = 1.0 / harmonic as f32;
                tone += weight * (phase * harmonic as f32).sin();
            }
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let noise = (state as f32 / u32::MAX as f32) * 2.0 - 1.0;
            let syllable = (2.0 * std::f32::consts::PI * 4.0 * t + seed as f32).sin();
            let envelope = syllable.max(0.0).powi(2);
            0.05 * envelope * (tone + 0.2 * noise)
        })
        .collect()
}

fn delayed(input: &[f32], delay_samples: usize, gain: f32) -> Vec<f32> {
    let mut output = vec![0.0; input.len()];
    for idx in delay_samples..input.len() {
        output[idx] = gain * input[idx - delay_samples];
    }
    output
}

fn add(left: &[f32], right: &[f32]) -> Vec<f32> {
    left.iter().zip(right).map(|(a, b)| a + b).collect()
}

fn energy(signal: &[f32]) -> f64 {
    signal.iter().map(|&x| f64::from(x) * f64::from(x)).sum()
}

fn db(ratio: f64) -> f64 {
    10.0 * ratio.max(1e-20).log10()
}

/// The energy of the part of `signal` that is a scaled copy of `reference`.
fn projected_energy(signal: &[f32], reference: &[f32]) -> f64 {
    let cross: f64 = signal
        .iter()
        .zip(reference)
        .map(|(&a, &b)| f64::from(a) * f64::from(b))
        .sum();
    cross * cross / energy(reference).max(1e-20)
}

/// Runs the canceller in chunks of 100 ms, as Assemble does.
fn cancel(mic: &[f32], reference: &[f32]) -> Vec<f32> {
    let chunk = BLOCK_SHIFT * 12;
    let mut canceller = EchoCanceller::new().expect("the canceller loads");
    let mut output = Vec::with_capacity(mic.len());
    for (mic, reference) in mic.chunks(chunk).zip(reference.chunks(chunk)) {
        let whole = mic.len() - mic.len() % BLOCK_SHIFT;
        output.extend(
            canceller
                .cancel_chunk(&mic[..whole], &reference[..whole])
                .expect("the canceller runs"),
        );
    }
    output
}

#[test]
fn canceller_removes_a_delayed_scaled_reference() {
    let len = RATE * 10;
    let reference = voice(len, 180.0, 7);
    let mut near = voice(len, 120.0, 3);
    // The first half has echo only. The second half has near speech too.
    near[..len / 2].fill(0.0);
    let echo = delayed(&reference, 32, 0.5);
    let mic = add(&near, &echo);

    let output = cancel(&mic, &reference);
    let lag = aec::STREAMING_LATENCY_SAMPLES;
    assert!(output.iter().all(|x| x.is_finite()));

    // Echo only, after 1 s of model state. The output is late by `lag`.
    let (start, end) = (RATE, len / 2 - RATE / 4);
    let echo_only_loss = db(energy(&echo[start..end]) / energy(&output[start + lag..end + lag]));
    println!("echo only: {echo_only_loss:.1} dB");
    assert!(
        echo_only_loss >= 15.0,
        "echo only: the reference lost {echo_only_loss:.1} dB"
    );

    // Near speech over echo: measure the part of the output that is a copy of
    // the echo.
    let (start, end) = (len / 2 + RATE / 4, len - RATE / 4);
    let double_talk_loss = db(energy(&echo[start..end])
        / projected_energy(&output[start + lag..end + lag], &echo[start..end]));
    println!("double talk: {double_talk_loss:.1} dB");
    assert!(
        double_talk_loss >= 15.0,
        "double talk: the reference lost {double_talk_loss:.1} dB"
    );

    // The near speech stays in the output.
    let near_change = db(energy(&output[start + lag..end + lag]) / energy(&near[start..end]));
    println!("double talk: near speech level change {near_change:.2} dB");
    assert!(
        near_change.abs() < 3.0,
        "double talk: the near speech changed by {near_change:.2} dB"
    );
}

#[test]
fn canceller_passes_a_microphone_without_the_reference() {
    let len = RATE * 10;
    let reference = voice(len, 180.0, 7);
    let mic = voice(len, 120.0, 3);

    let output = cancel(&mic, &reference);
    let lag = aec::STREAMING_LATENCY_SAMPLES;

    let (start, end) = (RATE, len - RATE / 4);
    let change = db(energy(&output[start + lag..end + lag]) / energy(&mic[start..end]));
    println!("level change: {change:.2} dB");
    assert!(change.abs() < 1.0, "the level changed by {change:.2} dB");
}

#[test]
fn canceller_passes_a_microphone_with_a_silent_reference() {
    let len = RATE * 5;
    let reference = vec![0.0; len];
    let mic = voice(len, 120.0, 3);

    let output = cancel(&mic, &reference);
    let lag = aec::STREAMING_LATENCY_SAMPLES;

    let (start, end) = (RATE, len - RATE / 4);
    let change = db(energy(&output[start + lag..end + lag]) / energy(&mic[start..end]));
    println!("level change: {change:.2} dB");
    assert!(change.abs() < 1.0, "the level changed by {change:.2} dB");
}

#[test]
fn chunk_output_has_the_chunk_length() {
    let mut canceller = EchoCanceller::new().unwrap();
    let mic = voice(BLOCK_SHIFT * 5, 120.0, 3);
    let reference = voice(BLOCK_SHIFT * 5, 180.0, 7);
    let output = canceller.cancel_chunk(&mic, &reference).unwrap();
    assert_eq!(output.len(), mic.len());
}

#[test]
fn chunk_that_is_not_whole_blocks_is_refused() {
    let mut canceller = EchoCanceller::new().unwrap();
    let mic = vec![0.0; BLOCK_SHIFT + 1];
    let error = canceller.cancel_chunk(&mic, &mic).unwrap_err();
    assert!(matches!(error, aec::Error::ChunkLength { .. }), "{error}");
}

#[test]
fn chunks_of_different_lengths_are_refused() {
    let mut canceller = EchoCanceller::new().unwrap();
    let error = canceller
        .cancel_chunk(&vec![0.0; BLOCK_SHIFT * 2], &vec![0.0; BLOCK_SHIFT])
        .unwrap_err();
    assert!(matches!(error, aec::Error::ChunkLength { .. }), "{error}");
}

#[test]
fn reset_gives_the_same_output_as_a_new_canceller() {
    let mic = voice(RATE, 120.0, 3);
    let reference = voice(RATE, 180.0, 7);
    let whole = mic.len() - mic.len() % BLOCK_SHIFT;

    let mut canceller = EchoCanceller::new().unwrap();
    let first = canceller
        .cancel_chunk(&mic[..whole], &reference[..whole])
        .unwrap();
    canceller.reset();
    let second = canceller
        .cancel_chunk(&mic[..whole], &reference[..whole])
        .unwrap();
    assert_eq!(first, second);
}

/// Measures the real-time factor on 30 minutes of 16 kHz audio. Run it with
/// `cargo test -p aec --release -- --ignored --nocapture real_time_factor`.
#[test]
#[ignore = "slow: measures 30 minutes of audio"]
fn real_time_factor() {
    let minute = voice(RATE * 60, 120.0, 3);
    let reference = voice(RATE * 60, 180.0, 7);
    let mic = add(&minute, &delayed(&reference, 32, 0.5));
    let chunk = BLOCK_SHIFT * 125;
    let mut canceller = EchoCanceller::new().unwrap();

    let started = std::time::Instant::now();
    for _ in 0..30 {
        for (mic, reference) in mic.chunks(chunk).zip(reference.chunks(chunk)) {
            let whole = mic.len() - mic.len() % BLOCK_SHIFT;
            canceller
                .cancel_chunk(&mic[..whole], &reference[..whole])
                .unwrap();
        }
    }
    let elapsed = started.elapsed();
    let factor = elapsed.as_secs_f64() / (30.0 * 60.0);
    println!(
        "30 min of audio in {:.1} s, real-time factor {factor:.4}",
        elapsed.as_secs_f64()
    );
    assert!(elapsed.as_secs() < 60, "took {elapsed:?}");
}
