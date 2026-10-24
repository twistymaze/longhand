#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Ort(#[from] ort::Error),

    #[error("failed to read model from path: {0}")]
    ReadModelFromPath(#[from] std::io::Error),
}

// A failed builder step returns the builder with the error. Keep only the
// error: callers never reuse the builder.
impl From<ort::Error<ort::session::builder::SessionBuilder>> for Error {
    fn from(error: ort::Error<ort::session::builder::SessionBuilder>) -> Self {
        Self::Ort(error.into())
    }
}
