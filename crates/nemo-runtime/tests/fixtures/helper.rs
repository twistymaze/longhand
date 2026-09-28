//! Deterministic child process used only by the opt-in protocol tests.
use nemo_runtime::{Frame, RUNTIME_REVISION, read_frame, write_frame};
use serde_json::json;
use std::io::{self, Write};
fn main() {
    let frame = read_frame(&mut io::stdin()).unwrap();
    let mode = frame.metadata["mode"].as_str().unwrap();
    let generation = frame.metadata["generation"].as_u64().unwrap();
    let epoch = frame.metadata["epoch"].as_u64().unwrap();
    let mut metadata = json!({"version":1,"generation":generation,"epoch":epoch,"seq":frame.metadata["seq"],"event":"hello","runtime_revision":RUNTIME_REVISION});
    match mode {
        "version" => metadata["version"] = 9.into(),
        "sequence" => metadata["seq"] = 99.into(),
        "generation" => metadata["generation"] = (generation + 1).into(),
        "revision" => metadata["runtime_revision"] = "wrong".into(),
        "truncated" => {
            io::stdout().write_all(&[1, 0, 0]).unwrap();
            return;
        }
        "exit" => return,
        "full_stderr" => {
            io::stderr()
                .write_all(&vec![b'x'; 2 * 1024 * 1024])
                .unwrap();
        }
        "blocked_input" => loop {
            std::thread::park();
        },
        _ => {}
    }
    write_frame(
        &mut io::stdout(),
        &Frame {
            metadata: metadata.clone(),
            pcm: vec![],
        },
    )
    .unwrap();
    if mode == "full_stdout" {
        metadata["event"] = "ack".into();
        for _ in 0..100_000 {
            if write_frame(
                &mut io::stdout(),
                &Frame {
                    metadata: metadata.clone(),
                    pcm: vec![],
                },
            )
            .is_err()
            {
                return;
            }
        }
    }
}
