//! Resumable, verified weight download (KTD19).
//!
//! The transport is Handy's `src/managers/model/download.rs`, detached from
//! its `ModelManager` and Tauri so it runs and tests without either; errors
//! are typed instead of `anyhow`, and the connect and stall limits are
//! parameters so a stall can be tested without waiting a minute. See
//! PROVENANCE.md.
//!
//! Robustness, in the order the failure modes appear:
//! - a partial already at the expected size is verified without a request;
//!   an oversized one is deleted
//! - connection setup and every body chunk are bounded and race the cancel
//!   token, so a wedged transfer neither hangs nor ignores a cancel
//! - a 200 to a Range request restarts from zero; a 206 must start exactly at
//!   our offset
//! - a server sending more than the expected size is cut off at the first
//!   excess byte
//! - the final bytes are checked against the catalog size and SHA-256, and a
//!   mismatch deletes them

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::catalog::CatalogModel;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub connect: Duration,
    /// No headers or body bytes for this long means the transfer is wedged.
    pub stall: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(15),
            stall: Duration::from_secs(60),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("no response or data for {0:?}; the partial download is kept and a retry resumes it")]
    Stalled(Duration),
    #[error("server returned HTTP {0}")]
    Http(u16),
    #[error("server misbehaved: {0}")]
    Protocol(String),
    #[error("download incomplete: expected {expected} bytes, got {actual}")]
    Incomplete { expected: u64, actual: u64 },
    #[error("{model}: the downloaded file does not match its catalog hash and was deleted")]
    HashMismatch { model: String },
    #[error(transparent)]
    Network(#[from] reqwest::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Completed,
    /// Cancelled by the caller; the partial is kept for resume.
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub model_id: String,
    pub downloaded: u64,
    pub total: u64,
}

pub enum Event<'a> {
    Progress(&'a Progress),
    VerificationStarted,
    VerificationCompleted,
}

pub fn partial_path(final_path: &Path) -> PathBuf {
    let mut name = final_path.file_name().unwrap_or_default().to_os_string();
    name.push(".partial");
    final_path.with_file_name(name)
}

/// Whether `model` is present in `dir` at its catalog size. The hash was
/// checked when it was downloaded; readiness does not re-hash 600 MB.
pub fn is_present(dir: &Path, model: &CatalogModel) -> bool {
    dir.join(&model.filename)
        .metadata()
        .is_ok_and(|m| m.len() == model.size_bytes)
}

/// Download `model` into `dir`, resuming any partial, and move it into place
/// only once it verifies.
pub async fn ensure_model(
    dir: &Path,
    model: &CatalogModel,
    limits: Limits,
    cancel: &CancellationToken,
    emit: &(dyn Fn(Event<'_>) + Send + Sync),
) -> Result<Outcome, DownloadError> {
    if is_present(dir, model) {
        return Ok(Outcome::Completed);
    }
    fs::create_dir_all(dir)?;
    let final_path = dir.join(&model.filename);
    let partial = partial_path(&final_path);
    let outcome = download_resumable(
        &model.id,
        &model.url,
        &partial,
        Some(model.size_bytes),
        Some(&model.sha256),
        limits,
        cancel,
        emit,
    )
    .await?;
    if outcome == Outcome::Completed {
        fs::rename(&partial, &final_path)?;
    }
    Ok(outcome)
}

/// A snapshot artifact has a separate directory for its exact hash.
/// This does not replace the current catalog file with the same name.
pub fn pinned_artifact_path(
    dir: &Path,
    artifact: &crate::recognition::ArtifactSpec,
) -> Result<PathBuf, DownloadError> {
    let hex = |value: &str, length| {
        value.len() == length
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    };
    let name = &artifact.filename;
    if !hex(&artifact.sha256, 64)
        || !hex(&artifact.revision, 40)
        || artifact.size_bytes == 0
        || name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\', ':'])
        || !artifact.url.starts_with("https://")
        || !artifact.url.contains(&artifact.revision)
    {
        return Err(DownloadError::Protocol(
            "invalid pinned artifact identity".into(),
        ));
    }
    Ok(dir.join("pinned").join(&artifact.sha256).join(name))
}

pub fn pinned_artifact_present(dir: &Path, artifact: &crate::recognition::ArtifactSpec) -> bool {
    pinned_artifact_path(dir, artifact).is_ok_and(|path| {
        path.metadata()
            .is_ok_and(|m| m.is_file() && m.len() == artifact.size_bytes)
    })
}

/// Download the snapshot's URL and hash, independently of the current catalog.
pub async fn ensure_artifact(
    dir: &Path,
    artifact: &crate::recognition::ArtifactSpec,
    limits: Limits,
    cancel: &CancellationToken,
    emit: &(dyn Fn(Event<'_>) + Send + Sync),
) -> Result<Outcome, DownloadError> {
    let path = pinned_artifact_path(dir, artifact)?;
    if pinned_artifact_present(dir, artifact) {
        return Ok(Outcome::Completed);
    }
    fs::create_dir_all(path.parent().expect("pinned artifact has a parent"))?;
    let partial = partial_path(&path);
    let outcome = download_resumable(
        &artifact.id,
        &artifact.url,
        &partial,
        Some(artifact.size_bytes),
        Some(&artifact.sha256),
        limits,
        cancel,
        emit,
    )
    .await?;
    if outcome == Outcome::Completed {
        fs::rename(&partial, &path)?;
    }
    Ok(outcome)
}

fn content_range_start(value: &str) -> Option<u64> {
    let range = value.trim().strip_prefix("bytes")?.trim_start();
    range.split('-').next()?.trim().parse().ok()
}

pub fn compute_sha256(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// On mismatch or read error the file is deleted, so the next attempt starts clean.
fn verify_sha256(path: &Path, expected: Option<&str>, model_id: &str) -> Result<(), DownloadError> {
    let Some(expected) = expected else {
        return Ok(());
    };
    match compute_sha256(path) {
        Ok(actual) if actual == expected => {
            log::info!("SHA-256 verified for {model_id}");
            Ok(())
        }
        Ok(actual) => {
            log::warn!("SHA-256 mismatch for {model_id}: expected {expected}, got {actual}");
            let _ = fs::remove_file(path);
            Err(DownloadError::HashMismatch {
                model: model_id.to_string(),
            })
        }
        Err(e) => {
            let _ = fs::remove_file(path);
            Err(e.into())
        }
    }
}

async fn verify_with_events(
    model_id: &str,
    path: &Path,
    expected: Option<&str>,
    emit: &(dyn Fn(Event<'_>) + Send + Sync),
) -> Result<(), DownloadError> {
    emit(Event::VerificationStarted);
    let (path, expected, id) = (
        path.to_path_buf(),
        expected.map(str::to_string),
        model_id.to_string(),
    );
    tokio::task::spawn_blocking(move || verify_sha256(&path, expected.as_deref(), &id))
        .await
        .map_err(|e| DownloadError::Io(std::io::Error::other(e)))??;
    emit(Event::VerificationCompleted);
    Ok(())
}

/// Fetch `url` into `partial_path`, resuming what is already there, and leave
/// verified bytes in `partial_path` on success. Finalizing is the caller's job.
#[allow(clippy::too_many_arguments)]
pub async fn download_resumable(
    model_id: &str,
    url: &str,
    partial_path: &Path,
    expected_size: Option<u64>,
    expected_sha256: Option<&str>,
    limits: Limits,
    cancel: &CancellationToken,
    emit: &(dyn Fn(Event<'_>) + Send + Sync),
) -> Result<Outcome, DownloadError> {
    let mut resume_from = partial_path.metadata().map(|m| m.len()).unwrap_or(0);

    if let Some(expected) = expected_size {
        if resume_from > expected {
            let _ = fs::remove_file(partial_path);
            resume_from = 0;
        } else if resume_from == expected && expected > 0 {
            log::info!("partial download of {model_id} is already full-size; verifying");
            verify_with_events(model_id, partial_path, expected_sha256, emit).await?;
            return Ok(Outcome::Completed);
        }
    }

    if resume_from > 0 {
        log::info!("resuming download of {model_id} from byte {resume_from}");
    } else {
        log::info!("starting download of {model_id} from {url}");
    }

    let client = reqwest::Client::builder()
        .connect_timeout(limits.connect)
        .build()?;
    let mut request = client.get(url);
    if resume_from > 0 {
        request = request.header("Range", format!("bytes={resume_from}-"));
    }
    let response = tokio::select! {
        r = tokio::time::timeout(limits.stall, request.send()) => {
            r.map_err(|_| DownloadError::Stalled(limits.stall))??
        }
        _ = cancel.cancelled() => return Ok(Outcome::Cancelled),
    };

    // 416 to a Range request: with a catalog size in hand the server's object
    // is smaller than expected, and without a hash nothing can bless the
    // partial. Only a hash can genuinely finish a partial here.
    if resume_from > 0 && response.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        if expected_size.is_some() || expected_sha256.is_none() {
            let _ = fs::remove_file(partial_path);
            return Err(DownloadError::Protocol(
                "server object ends before the expected size (HTTP 416)".into(),
            ));
        }
        verify_with_events(model_id, partial_path, expected_sha256, emit).await?;
        return Ok(Outcome::Completed);
    }
    // A 200 to a Range request: the server is sending the whole file.
    if resume_from > 0 && response.status() == reqwest::StatusCode::OK {
        let _ = fs::remove_file(partial_path);
        resume_from = 0;
    }
    if !response.status().is_success() {
        return Err(DownloadError::Http(response.status().as_u16()));
    }
    if resume_from > 0 && response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        let starts_at = response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(content_range_start);
        if starts_at != Some(resume_from) {
            let _ = fs::remove_file(partial_path);
            return Err(DownloadError::Protocol(format!(
                "Content-Range starts at {starts_at:?}, expected {resume_from}"
            )));
        }
    }
    if let (Some(expected), Some(len)) = (expected_size, response.content_length())
        && resume_from + len != expected
    {
        return Err(DownloadError::Protocol(format!(
            "server advertises {} bytes, expected {expected}",
            resume_from + len
        )));
    }

    let known_total = expected_size.or_else(|| response.content_length().map(|l| resume_from + l));
    let total = known_total.unwrap_or(0);
    let mut downloaded = resume_from;
    let mut file = if resume_from > 0 {
        fs::OpenOptions::new().append(true).open(partial_path)?
    } else {
        File::create(partial_path)?
    };

    let emit_progress = |downloaded: u64| {
        emit(Event::Progress(&Progress {
            model_id: model_id.to_string(),
            downloaded,
            total,
        }));
    };
    emit_progress(downloaded);

    let mut last_emit = Instant::now();
    let throttle = Duration::from_millis(100);
    let mut stream = response.bytes_stream();
    loop {
        let chunk = tokio::select! {
            c = tokio::time::timeout(limits.stall, stream.next()) => match c {
                Err(_) => {
                    file.flush()?;
                    return Err(DownloadError::Stalled(limits.stall));
                }
                Ok(None) => break,
                Ok(Some(chunk)) => chunk?,
            },
            _ = cancel.cancelled() => {
                file.flush()?;
                return Ok(Outcome::Cancelled);
            }
        };
        if let Some(cap) = known_total
            && downloaded + chunk.len() as u64 > cap
        {
            drop(file);
            let _ = fs::remove_file(partial_path);
            return Err(DownloadError::Protocol(format!(
                "server sent more than the expected {cap} bytes"
            )));
        }
        file.write_all(&chunk)?;
        downloaded += chunk.len() as u64;
        if last_emit.elapsed() >= throttle {
            emit_progress(downloaded);
            last_emit = Instant::now();
        }
    }
    file.flush()?;
    drop(file);
    emit_progress(downloaded);

    if let Some(expected) = known_total {
        let actual = partial_path.metadata()?.len();
        if actual != expected {
            let _ = fs::remove_file(partial_path);
            return Err(DownloadError::Incomplete { expected, actual });
        }
    }

    verify_with_events(model_id, partial_path, expected_sha256, emit).await?;
    Ok(Outcome::Completed)
}

#[cfg(test)]
mod tests;
