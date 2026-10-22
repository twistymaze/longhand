//! Against the machine's real output. Ignored by default: CI has no audio
//! device, and a local run asks for the system audio permission. Run it from
//! the signed bundle's identity, with no other audio playing:
//!
//! `cargo test -p audio-system --test hardware -- --ignored --nocapture --test-threads=1`

#![cfg(any(target_os = "macos", target_os = "windows"))]

use std::time::Duration;

use futures_util::StreamExt;

/// A peak above this is audio, not the noise floor.
const FLOOR: f32 = 0.01;

/// Read system audio for `seconds` and return the highest peak.
async fn peak_for(seconds: f64) -> f32 {
    let capture = audio_system::open(48_000, 960).expect("system audio opens");
    let mut stream = capture.stream;
    let frames = (seconds * 50.0) as usize;
    let mut peak = 0.0_f32;
    for _ in 0..frames {
        let frame = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .expect("a frame within 2 s")
            .expect("stream still open")
            .expect("frame, not error");
        assert_eq!(frame.raw_speaker.len(), 960);
        assert!(frame.raw_mic.is_empty());
        peak = frame.raw_speaker.iter().fold(peak, |p, s| p.max(s.abs()));
    }
    assert!(!capture.status.other_output_playing());
    peak
}

#[cfg(target_os = "macos")]
fn tone_file() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("longhand-tone-{}.wav", std::process::id()));
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 48_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&path, spec).unwrap();
    for i in 0..48_000 * 3 {
        let t = i as f32 / 48_000.0;
        let s = (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.3;
        writer.write_sample((s * i16::MAX as f32) as i16).unwrap();
    }
    writer.finalize().unwrap();
    path
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "needs a real output device and the system audio permission"]
async fn a_tone_from_another_process_is_in_the_tap() {
    let path = tone_file();
    let mut player = std::process::Command::new("afplay")
        .arg(&path)
        .spawn()
        .expect("afplay starts");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let peak = peak_for(1.5).await;
    let _ = player.wait();
    let _ = std::fs::remove_file(&path);
    println!("peak with another process playing: {peak}");
    assert!(peak > FLOOR, "the tone is in the tap, peak {peak}");
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "needs a real output device and the system audio permission"]
async fn a_tone_from_this_process_is_not_in_the_tap() {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let device = cpal::default_host()
        .default_output_device()
        .expect("an output device");
    let config = device.default_output_config().unwrap();
    let rate = config.sample_rate() as f32;
    let channels = config.channels() as usize;
    let mut n = 0_u64;
    let stream = device
        .build_output_stream(
            config.config(),
            move |data: &mut [f32], _| {
                for frame in data.chunks_mut(channels) {
                    let t = n as f32 / rate;
                    let s = (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.3;
                    frame.fill(s);
                    n += 1;
                }
            },
            |e| eprintln!("output error: {e}"),
            None,
        )
        .expect("output stream builds");
    stream.play().unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let peak = peak_for(1.5).await;
    drop(stream);
    println!("peak with this process playing: {peak}");
    assert!(peak < FLOOR, "the tap excludes this process, peak {peak}");
}

/// AE2: an app that starts to play after the app tap opens is recorded.
/// Set `LONGHAND_TEST_APP` to the app's bundle ID (for example
/// `us.zoom.xos`), start the test, then start audio in that app within
/// 10 s.
#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "needs a real output device, the system audio permission and an app that plays"]
async fn an_app_that_starts_after_the_tap_opens_is_recorded() {
    let app = std::env::var("LONGHAND_TEST_APP").expect("LONGHAND_TEST_APP holds a bundle ID");
    let apps = audio_system::AppSet::new(vec![app.clone()]);
    let capture = audio_system::open_apps(48_000, 960, apps).expect("app audio opens");
    let mut stream = capture.stream;
    println!("Start audio in {app} now");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut peak = 0.0_f32;
    while peak <= FLOOR {
        let frame = tokio::time::timeout_at(deadline, stream.next())
            .await
            .expect("app audio within 10 s")
            .expect("stream still open")
            .expect("frame, not error");
        assert_eq!(frame.raw_speaker.len(), 960);
        peak = frame.raw_speaker.iter().fold(peak, |p, s| p.max(s.abs()));
    }
    println!("peak from {app}: {peak}");
}

#[cfg(target_os = "windows")]
#[tokio::test]
#[ignore = "needs a real output device and audio that plays during the test"]
async fn loopback_delivers_audio_while_the_output_plays() {
    println!("Play audio on the default output while this test runs");
    let peak = peak_for(3.0).await;
    println!("peak: {peak}");
    assert!(peak > FLOOR, "the loopback is silent, peak {peak}");
}
