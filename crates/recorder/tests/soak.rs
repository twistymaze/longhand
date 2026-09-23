//! The four-hour recorder soak (U21): 4 h at 48 kHz through the chunk
//! writer as fast as it accepts, while another thread keeps a core busy as
//! recognition would. Checks chunk accounting and that memory stays flat.
//! Writes about 1.4 GB.
//!
//! ```text
//! cargo test -p recorder --release --test soak -- --ignored --nocapture
//! ```
//!
//! Mock audio cannot meet a driver's deadlines, so ring overruns under load
//! are the real-microphone check at the Phase 4 checkpoint, not this one.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use recorder::storage::DISK_RESERVE_BYTES;
use recorder::{ChunkedWriter, SegmentArgs, list_chunks};

const RATE: u32 = 48_000;
const HOURS: u64 = 4;

fn resident_bytes() -> u64 {
    let pid = sysinfo::get_current_pid().unwrap();
    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    system.process(pid).map(|p| p.memory()).unwrap_or(0)
}

#[test]
#[ignore = "writes 1.4 GB"]
fn soak_four_hours_beside_a_busy_core() {
    let dir = tempfile::tempdir().unwrap();
    let mut writer = ChunkedWriter::with_probe(
        SegmentArgs {
            dir: dir.path().to_path_buf(),
            segment: 0,
            capture_started_at: 0,
            offset_ms: 0,
            sample_rate: RATE,
            floor_bytes: DISK_RESERVE_BYTES,
        },
        Box::new(|_| Some(u64::MAX)),
    )
    .unwrap();

    let done = Arc::new(AtomicBool::new(false));
    let busy = {
        let done = Arc::clone(&done);
        std::thread::spawn(move || {
            let mut x = 0u64;
            while !done.load(Ordering::Relaxed) {
                for i in 0..10_000u64 {
                    x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(i);
                }
            }
            x
        })
    };

    let block: Vec<f32> = (0..RATE as usize / 50)
        .map(|i| (i as f32 * 0.05).sin() * 0.3)
        .collect();
    let blocks = HOURS * 3_600 * 50;
    let started = Instant::now();
    let mut baseline = 0;
    let mut peak = 0;
    for n in 0..blocks {
        writer.write(&block).unwrap();
        // Measured from the first minute on, once the writer's buffers and
        // the allocator have settled.
        if n % (50 * 60) == 0 {
            let rss = resident_bytes();
            if n == 50 * 60 {
                baseline = rss;
            }
            peak = peak.max(rss);
        }
    }
    let span = writer.finish().unwrap();
    done.store(true, Ordering::Relaxed);
    busy.join().unwrap();

    let chunks = list_chunks(dir.path()).unwrap();
    println!(
        "{} chunks in {:.1} s; resident {} MB at one minute, {} MB at peak",
        chunks.len(),
        started.elapsed().as_secs_f32(),
        baseline / 1_000_000,
        peak / 1_000_000
    );
    assert_eq!(span.end_ms, HOURS * 3_600_000);
    assert_eq!(chunks.len() as u64, HOURS * 60);
    assert!(chunks.windows(2).all(|p| p[0].end_ms == p[1].start_ms));
    assert!(
        peak.saturating_sub(baseline) < 50_000_000,
        "resident memory grew {} MB",
        (peak - baseline) / 1_000_000
    );
}
