#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Onnx(#[from] ::onnx::Error),

    #[error(transparent)]
    Ort(#[from] ::onnx::ort::Error),

    #[error(transparent)]
    Fft(#[from] realfft::FftError),

    #[error(transparent)]
    Shape(#[from] ::onnx::ndarray::ShapeError),

    #[error("the model gave no output tensor {0}")]
    MissingOutput(String),

    #[error(
        "the chunk has {mic} microphone samples and {reference} reference samples; \
         both must be equal and a multiple of {}",
        crate::BLOCK_SHIFT
    )]
    ChunkLength { mic: usize, reference: usize },
}
