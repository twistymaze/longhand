//! The WAV-backed audio source.

use diarize::{AudioSource, DiarizeError, WavAudioSource};

fn write(path: &std::path::Path, rate: u32, channels: u16, float: bool, samples: &[f32]) {
    let spec = hound::WavSpec {
        channels,
        sample_rate: rate,
        bits_per_sample: if float { 32 } else { 16 },
        sample_format: if float {
            hound::SampleFormat::Float
        } else {
            hound::SampleFormat::Int
        },
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for &s in samples {
        if float {
            w.write_sample(s).unwrap();
        } else {
            w.write_sample((s * 32_767.0) as i16).unwrap();
        }
    }
    w.finalize().unwrap();
}

#[test]
fn a_44_1_khz_stereo_file_is_a_typed_format_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("stereo.wav");
    write(&path, 44_100, 2, false, &[0.0; 4_410]);
    assert!(matches!(
        WavAudioSource::open(&path),
        Err(DiarizeError::Format(_))
    ));
}

#[test]
fn a_read_past_the_end_is_real_audio_then_zeros() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("proc.wav");
    let rate = 16_000usize;
    // 100 s whose samples encode their own second, so position is checkable.
    let samples: Vec<f32> = (0..100 * rate).map(|i| (i / rate) as f32 / 100.0).collect();
    write(&path, rate as u32, 1, true, &samples);

    let mut source = WavAudioSource::open(&path).unwrap();
    assert_eq!(source.sample_count().unwrap(), 100 * rate);
    let mut out = vec![1.0f32; 5 * rate];
    source.read(99 * rate, &mut out).unwrap();
    assert!(out[..rate].iter().all(|&s| (s - 0.99).abs() < 1e-6));
    assert!(out[rate..].iter().all(|&s| s == 0.0));

    // Entirely past the end reads as silence rather than an error.
    source.read(200 * rate, &mut out).unwrap();
    assert!(out.iter().all(|&s| s == 0.0));
}
