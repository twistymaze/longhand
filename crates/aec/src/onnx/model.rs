// The DTLN-aec model at the 128-unit size. Stage 1 estimates a spectral mask,
// stage 2 refines the masked block in the time domain.
pub const BYTES_1: &[u8] = include_bytes!("../../data/model_128_1.onnx");
pub const BYTES_2: &[u8] = include_bytes!("../../data/model_128_2.onnx");
pub const STATE_SIZE: usize = 128;

// The model was trained with these values.
pub const BLOCK_SIZE: usize = 512;
pub const BLOCK_SHIFT: usize = 128;
