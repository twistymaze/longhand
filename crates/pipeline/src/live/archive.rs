//! Read the process-crash-safe prefix of capture files, without using unfinished
//! WAV lengths. Keep one open handle across flushes and chunk renames.
use super::LiveFrame;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    sync::Arc,
};

#[derive(Debug, Clone)]
pub struct Archive {
    pub dir: PathBuf,
    pub segment: u32,
    pub offset_ms: u64,
    pub rate: u32,
}
struct OpenChunk {
    file: File,
    audio_start: u64,
    channels: u16,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SourceProof {
    pub samples: u64,
    pub rate: u32,
    pub channels: u16,
    pub digest: String,
}
pub struct ArchiveReader {
    archive: Archive,
    next: u64,
    active: Option<OpenChunk>,
    logical_start: Option<u64>,
    channels: u16,
    digest: Sha256,
}
impl ArchiveReader {
    pub fn new(archive: Archive) -> Self {
        Self {
            archive,
            next: 0,
            active: None,
            logical_start: None,
            channels: 0,
            digest: Sha256::new(),
        }
    }
    pub fn position(&self) -> u64 {
        self.next
    }
    pub fn proof(&self) -> SourceProof {
        SourceProof {
            samples: self.next,
            rate: self.archive.rate,
            channels: self.channels,
            digest: format!("{:x}", self.digest.clone().finalize()),
        }
    }
    fn read_active(&mut self) -> std::io::Result<Option<LiveFrame>> {
        let Some(chunk) = self.active.as_mut() else {
            return Ok(None);
        };
        let width = u64::from(chunk.channels) * 2;
        let frames = chunk.file.metadata()?.len().saturating_sub(44) / width;
        let end = chunk.audio_start + frames;
        if self.next >= end {
            return Ok(None);
        }
        let count = (end - self.next)
            .min(u64::from(self.archive.rate) / 50)
            .max(1) as usize;
        chunk.file.seek(SeekFrom::Start(
            44 + (self.next - chunk.audio_start) * width,
        ))?;
        let mut bytes = vec![0; count * width as usize];
        chunk.file.read_exact(&mut bytes)?;
        self.digest.update(&bytes);
        let samples: Vec<f32> = bytes
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
            .collect();
        let pcm: Arc<[f32]> = if chunk.channels == 2 {
            let left: Vec<_> = samples.iter().step_by(2).copied().collect();
            let right: Vec<_> = samples.iter().skip(1).step_by(2).copied().collect();
            audio::live_mix::mix(&left, &right).into()
        } else {
            samples.into()
        };
        let frame = LiveFrame {
            index: self.next,
            pcm,
        };
        self.next += count as u64;
        Ok(Some(frame))
    }
    /// None means that the recorder has not flushed the next whole frame yet.
    pub fn next_frame(&mut self) -> std::io::Result<Option<LiveFrame>> {
        if let Some(frame) = self.read_active()? {
            return Ok(Some(frame));
        }
        let mut candidates = Vec::new();
        for entry in std::fs::read_dir(&self.archive.dir)? {
            let path = entry?.path();
            let extension = path.extension().and_then(|v| v.to_str());
            if !matches!(extension, Some("part" | "wav")) {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Some(fields) = stem
                .split('-')
                .map(|v| v.parse::<u64>().ok())
                .collect::<Option<Vec<_>>>()
            else {
                continue;
            };
            let expected = if extension == Some("part") { 4 } else { 5 };
            if fields.len() != expected || fields[0] != u64::from(self.archive.segment) {
                continue;
            }
            let logical = fields[2];
            let audio = fields[expected - 1];
            if logical < self.archive.offset_ms
                || audio < self.archive.offset_ms
                || self.logical_start.is_some_and(|old| logical <= old)
            {
                continue;
            }
            candidates.push((logical, audio, path));
        }
        candidates.sort_by_key(|(logical, _, _)| *logical);
        let rate = u64::from(self.archive.rate);
        for (logical, audio, path) in candidates {
            let start = (logical - self.archive.offset_ms) * rate / 1000;
            if start > self.next {
                return Ok(None);
            }
            let audio = (audio - self.archive.offset_ms) * rate / 1000;
            let mut file = match File::open(&path) {
                Ok(file) => file,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            if file.metadata()?.len() < 44 {
                continue;
            }
            let mut header = [0u8; 44];
            file.read_exact(&mut header)?;
            let channels = u16::from_le_bytes([header[22], header[23]]);
            let sample_rate = u32::from_le_bytes(header[24..28].try_into().unwrap());
            let align = u16::from_le_bytes([header[32], header[33]]);
            let byte_rate = u32::from_le_bytes(header[28..32].try_into().unwrap());
            if &header[..4] != b"RIFF"
                || &header[8..16] != b"WAVEfmt "
                || header[16..20] != [16, 0, 0, 0]
                || header[20..22] != [1, 0]
                || &header[36..40] != b"data"
                || !matches!(channels, 1 | 2)
                || sample_rate != self.archive.rate
                || header[34..36] != [16, 0]
                || align != channels * 2
                || byte_rate != sample_rate * u32::from(align)
                || (self.channels != 0 && self.channels != channels)
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "invalid capture audio format",
                ));
            }
            self.channels = channels;
            self.active = Some(OpenChunk {
                file,
                audio_start: audio,
                channels,
            });
            self.logical_start = Some(logical);
            return self.read_active();
        }
        Ok(None)
    }
}
pub fn key(segment: u32) -> String {
    format!("live-source-{segment:08}")
}

pub fn save_proof(
    store: &crate::SharedStore,
    config: &super::LiveConfig,
    reader: &ArchiveReader,
) -> Result<(), crate::StageError> {
    let proof = reader.proof();
    let guard = crate::lock(store);
    let previous: Option<SourceProof> = guard
        .checkpoint_value(
            &config.note_id,
            session_core::Stage::Transcribe,
            &key(config.segment),
        )?
        .map(|value| serde_json::from_str(&value))
        .transpose()?;
    if previous.is_none_or(|old| old.samples <= proof.samples) {
        guard.put_checkpoint(
            &config.note_id,
            session_core::Stage::Transcribe,
            &key(config.segment),
            &serde_json::to_string(&proof)?,
        )?;
        if let Some(spec) = &config.snapshot {
            guard.put_checkpoint(
                &config.note_id,
                session_core::Stage::Transcribe,
                &format!("live-snapshot-{:08}", config.segment),
                &serde_json::to_string(spec)?,
            )?;
        }
    }
    Ok(())
}
