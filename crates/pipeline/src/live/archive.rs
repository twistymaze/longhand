//! Read the process-crash-safe prefix of capture files, without using unfinished
//! WAV lengths. Keep one open handle across flushes and chunk renames.
//!
//! A part of a cut segment starts at a chunk whose lookback lies before
//! the segment offset. The reader skips those samples (KTD16 of the
//! forgotten-recording plan).
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
    /// The position of the first frame of the file. It is negative when
    /// the lookback of the file starts before the archive offset.
    audio_start: i64,
    channels: u16,
}

/// The running digest of one range of frames (KTD18 of the
/// forgotten-recording plan).
struct PartDigest {
    range: std::ops::Range<u64>,
    samples: u64,
    digest: Sha256,
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
    parts: Vec<PartDigest>,
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
            parts: Vec::new(),
        }
    }
    /// A reader that also keeps one digest for each range of frames in
    /// `parts`. A range must start and end at a chunk boundary, because a
    /// read frame never crosses the end of a chunk.
    pub fn with_parts(archive: Archive, parts: Vec<std::ops::Range<u64>>) -> Self {
        let mut reader = Self::new(archive);
        reader.parts = parts
            .into_iter()
            .map(|range| PartDigest {
                range,
                samples: 0,
                digest: Sha256::new(),
            })
            .collect();
        reader
    }
    /// The proof of each range of [`Self::with_parts`]: the proof that a
    /// reader from the start of the range gives after the frames of the
    /// range that this reader read.
    pub fn part_proofs(&self) -> Vec<SourceProof> {
        self.parts
            .iter()
            .map(|part| SourceProof {
                samples: part.samples,
                rate: self.archive.rate,
                channels: self.channels,
                digest: format!("{:x}", part.digest.clone().finalize()),
            })
            .collect()
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
        let end = chunk.audio_start + frames as i64;
        if self.next as i64 >= end {
            return Ok(None);
        }
        let count = (end - self.next as i64)
            .min(i64::from(self.archive.rate) / 50)
            .max(1) as usize;
        let skip = (self.next as i64 - chunk.audio_start) as u64;
        chunk.file.seek(SeekFrom::Start(44 + skip * width))?;
        let mut bytes = vec![0; count * width as usize];
        chunk.file.read_exact(&mut bytes)?;
        self.digest.update(&bytes);
        for part in &mut self.parts {
            if part.range.contains(&self.next) {
                part.digest.update(&bytes);
                part.samples += count as u64;
            }
        }
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
                || audio > logical
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
            // The lookback before the offset is skipped, as Assemble skips
            // it.
            let audio = if audio >= self.archive.offset_ms {
                ((audio - self.archive.offset_ms) * rate / 1000) as i64
            } else {
                -(((self.archive.offset_ms - audio) * rate / 1000) as i64)
            };
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Write `seconds` of 16-kHz audio as segment 0 at the offset 0: one
    /// chunk for each 60 s, each later chunk with a lookback of 2 s.
    fn segment(dir: &std::path::Path, seconds: usize) {
        let mut writer = recorder::ChunkedWriter::with_probe(
            recorder::SegmentArgs {
                dir: dir.to_path_buf(),
                segment: 0,
                capture_started_at: 1_800_000_000_000,
                offset_ms: 0,
                sample_rate: 16_000,
                channels: 1,
                floor_bytes: 0,
            },
            Box::new(|_| Some(u64::MAX)),
        )
        .unwrap();
        let audio: Vec<f32> = (0..seconds * 16_000)
            .map(|i| ((i * 7_919 % 2_003) as f32 / 2_003.0 - 0.5) * 0.8)
            .collect();
        for block in audio.chunks(320) {
            writer.write(block).unwrap();
        }
        writer.finish().unwrap();
    }

    fn archive(dir: &std::path::Path, segment: u32, offset_ms: u64) -> Archive {
        Archive {
            dir: dir.to_path_buf(),
            segment,
            offset_ms,
            rate: 16_000,
        }
    }

    fn read_all(reader: &mut ArchiveReader, samples: u64) {
        while reader.position() < samples && reader.next_frame().unwrap().is_some() {}
    }

    #[test]
    fn a_part_proof_is_the_proof_of_a_reader_from_the_part_start() {
        let dir = tempfile::tempdir().unwrap();
        segment(dir.path(), 180);
        // Part A is the first chunk, part B the last chunk. The cut removes
        // the second chunk.
        let mut whole = ArchiveReader::with_parts(
            archive(dir.path(), 0, 0),
            vec![0..960_000, 1_920_000..u64::MAX],
        );
        read_all(&mut whole, u64::MAX);
        assert_eq!(whole.position(), 2_880_000);
        let proofs = whole.part_proofs();
        assert_eq!(proofs[0].samples, 960_000);
        assert_eq!(proofs[1].samples, 960_000);
        // The chunk of part B gets the index 1, as the cut renames it.
        for entry in std::fs::read_dir(dir.path()).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if name.starts_with("0-") && name.contains("-120000-") {
                std::fs::rename(&path, dir.path().join(format!("1{}", &name[1..]))).unwrap();
            }
        }
        let mut part = ArchiveReader::new(archive(dir.path(), 1, 120_000));
        read_all(&mut part, proofs[1].samples);
        assert_eq!(part.proof(), proofs[1]);
        let mut part = ArchiveReader::new(archive(dir.path(), 0, 0));
        read_all(&mut part, proofs[0].samples);
        assert_eq!(part.proof(), proofs[0]);
    }

    #[test]
    fn a_part_proof_covers_only_the_frames_that_the_reader_read() {
        let dir = tempfile::tempdir().unwrap();
        segment(dir.path(), 180);
        // The live worker was behind: it read 130 s.
        let part = 1_920_000..u64::MAX;
        let mut whole = ArchiveReader::with_parts(archive(dir.path(), 0, 0), vec![part]);
        read_all(&mut whole, 2_080_000);
        assert_eq!(whole.part_proofs()[0].samples, 160_000);
    }
}
