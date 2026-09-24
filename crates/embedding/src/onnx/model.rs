//! The WeSpeaker ResNet34-LM ONNX model's interface. The weights are not
//! compiled in: they are downloaded from their upstream release (R28) and
//! loaded from the models directory.

pub const INPUT_NAME: &str = "feats";
pub const OUTPUT_NAME: &str = "embs";

pub const SAMPLE_RATE_HZ: u32 = 16000;
pub const NUM_MEL_BINS: usize = 80;
pub const EMBEDDING_DIM: usize = 256;
