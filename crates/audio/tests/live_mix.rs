//! The plain mix of a stereo segment (KTD9 of the app audio plan): the sum
//! of the microphone (left) and the app audio (right), clamped to the
//! sample range.

use audio::live_mix::mix;

const RATE: usize = 48_000;

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

#[test]
fn left_0_25_and_right_0_5_give_0_75() {
    assert_eq!(mix(&[0.25], &[0.5]), [0.75]);
}

#[test]
fn a_sum_outside_the_sample_range_is_clamped() {
    assert_eq!(mix(&[0.75, -0.75], &[0.75, -0.75]), [1.0, -1.0]);
}

#[test]
fn a_silent_right_channel_gives_the_left_channel_unchanged() {
    let left = scaled(&signal(RATE * 4, 0x1234_5678), 0.2);
    assert_eq!(mix(&left, &vec![0.0; left.len()]), left);
}

#[test]
fn two_channels_with_speech_give_their_plain_sum() {
    // A loud room and a quiet call: the mix changes neither level.
    let len = RATE * 20;
    let left = scaled(&signal(len, 0x1234_5678), 1.5);
    let right = scaled(&signal(len, 0x0bad_cafe), 0.0135);
    let out = mix(&left, &right);
    assert_eq!(out.len(), len);
    for (i, ((o, l), r)) in out.iter().zip(&left).zip(&right).enumerate() {
        assert_eq!(*o, (l + r).clamp(-1.0, 1.0), "sample {i}");
    }
}
