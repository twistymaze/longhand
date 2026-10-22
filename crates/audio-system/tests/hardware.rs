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

/// U1: print each audio process and the app that owns its sound. Run it
/// during a call to see which processes play the call.
#[cfg(target_os = "macos")]
#[test]
#[ignore = "prints the audio processes of this Mac"]
fn list_the_audio_processes_and_their_apps() {
    let processes = audio_system::apps::audio_processes();
    assert!(!processes.is_empty(), "the audio system lists processes");
    for p in &processes {
        println!(
            "object {:>5} pid {:>6} bundle {:<50} app {}",
            p.object,
            p.pid,
            p.bundle_id.as_deref().unwrap_or("-"),
            audio_system::apps::app_of(p).as_deref().unwrap_or("-"),
        );
    }
}

/// U1: print the peak of each second of an app's audio for 30 s. Run it
/// once while the app is idle and once during a call, to set the
/// threshold of the first app sound (KTD6). Set `LONGHAND_TEST_APP` to
/// the app's bundle ID.
#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "needs the system audio permission and a running app"]
async fn measure_the_peak_of_an_app_for_30_seconds() {
    let app = std::env::var("LONGHAND_TEST_APP").expect("LONGHAND_TEST_APP holds a bundle ID");
    let apps = audio_system::AppSet::new(vec![app.clone()]);
    let capture = audio_system::open_apps(48_000, 960, apps).expect("app audio opens");
    let mut stream = capture.stream;
    let end = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut second = tokio::time::Instant::now() + Duration::from_secs(1);
    let mut peak = 0.0_f32;
    let mut frames = 0;
    loop {
        match tokio::time::timeout_at(second.min(end), stream.next()).await {
            Ok(Some(Ok(frame))) => {
                frames += 1;
                peak = frame.raw_speaker.iter().fold(peak, |p, s| p.max(s.abs()));
                if tokio::time::Instant::now() < second {
                    continue;
                }
            }
            Ok(Some(Err(e))) => panic!("app audio ended: {e}"),
            Ok(None) => panic!("app audio stream closed"),
            Err(_) => {}
        }
        println!("{app}: {frames} frames, peak {peak:.5}");
        peak = 0.0;
        frames = 0;
        if tokio::time::Instant::now() >= end {
            break;
        }
        second += Duration::from_secs(1);
    }
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

/// Read app audio of `apps` for `seconds` and return the highest peak.
#[cfg(target_os = "windows")]
async fn app_peak_for(apps: &[&str], seconds: u64) -> f32 {
    let set = audio_system::AppSet::new(apps.iter().map(|a| (*a).to_owned()).collect());
    let capture = audio_system::open_apps(48_000, 960, set).expect("app audio opens");
    let mut stream = capture.stream;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let mut peak = 0.0_f32;
    // No frame arrives while no app in the set plays, so the deadline ends
    // the read.
    while let Ok(Some(frame)) = tokio::time::timeout_at(deadline, stream.next()).await {
        let frame = frame.expect("frame, not error");
        assert_eq!(frame.raw_speaker.len(), 960);
        peak = frame.raw_speaker.iter().fold(peak, |p, s| p.max(s.abs()));
    }
    peak
}

/// AE8: with Teams and Zoom in the set, a Teams call is recorded through
/// device loopback (Teams mode). Start a Teams call with sound before the
/// test. The log shows "Teams plays; app audio records all sound of ...".
#[cfg(target_os = "windows")]
#[tokio::test]
#[ignore = "needs Windows 11 and a Teams call that plays sound"]
async fn a_teams_call_is_recorded_in_teams_mode() {
    let peak = app_peak_for(&["ms-teams", "zoom"], 10).await;
    println!("peak of the Teams call: {peak}");
    assert!(peak > FLOOR, "the Teams call is silent, peak {peak}");
}

/// AE8: a Zoom call is recorded through per-process capture. Start a Zoom
/// call with sound before the test, with Teams closed or silent.
#[cfg(target_os = "windows")]
#[tokio::test]
#[ignore = "needs Windows 11 and a Zoom call that plays sound"]
async fn a_zoom_call_is_recorded_through_per_process_capture() {
    let peak = app_peak_for(&["zoom"], 10).await;
    println!("peak of the Zoom call: {peak}");
    assert!(peak > FLOOR, "the Zoom call is silent, peak {peak}");
}

/// U1 step 3: print each render session with its state, peak meter,
/// process ID, executable and parent chain. Run it while Teams is idle,
/// during a Teams call, and after the call.
#[cfg(target_os = "windows")]
#[test]
#[ignore = "prints the render sessions of this computer"]
fn list_the_render_sessions_and_their_parent_chains() {
    use audio_system::windows_app_audio::{parent_chain, render_sessions};

    let sessions = render_sessions();
    assert!(!sessions.is_empty(), "the audio system lists sessions");
    for s in &sessions {
        let chain: Vec<String> = parent_chain(s.pid)
            .iter()
            .map(|(pid, app)| format!("{app} ({pid})"))
            .collect();
        println!(
            "{:<30} {:<8} peak {:.4} pid {:>6} chain {}",
            s.endpoint_name.as_deref().unwrap_or(&s.endpoint_id),
            s.state,
            s.peak,
            s.pid,
            chain.join(" <- "),
        );
    }
}
