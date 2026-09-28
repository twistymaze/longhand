use nemo_runtime::{Frame, MAX_JSON_BYTES, read_frame, write_frame};
use serde_json::json;
use std::io::Cursor;

#[test]
fn rejects_truncated_and_oversized_frames() {
    assert!(read_frame(&mut Cursor::new([1, 0, 0])).is_err());
    let mut bytes = Vec::from(((MAX_JSON_BYTES + 1) as u32).to_le_bytes());
    bytes.extend(0_u32.to_le_bytes());
    assert!(read_frame(&mut Cursor::new(bytes)).is_err());
    assert!(read_frame(&mut Cursor::new([1, 0, 0, 0, 4, 0, 0, 0, b'{'])).is_err());
}

#[test]
fn round_trip_keeps_pcm_separate_from_metadata() {
    let frame = Frame {
        metadata: json!({"version": 1, "generation": 8}),
        pcm: vec![0.25, -0.5],
    };
    let mut bytes = Vec::new();
    write_frame(&mut bytes, &frame).unwrap();
    let result = read_frame(&mut Cursor::new(bytes)).unwrap();
    assert_eq!(result.metadata, frame.metadata);
    assert_eq!(result.pcm, frame.pcm);
}
