//! Model weights: the compiled-in catalog and the resumable, verified
//! download that fetches them from upstream (KTD19, R28).

pub mod catalog;
pub mod download;

pub use catalog::{
    CATALOG, CatalogModel, DEFAULT_ASR_MODEL, Licence, ModelKind, model, required_for,
};
pub use download::{DownloadError, Limits, Outcome, ensure_model, is_present};
pub use tokio_util::sync::CancellationToken;
