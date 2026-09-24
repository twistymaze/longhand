//! 16-bit PCM mono WAV, written by hand so the header can be repaired from
//! the file's byte length after a crash (KTD4) — anarlog measures a partial
//! MP3 by decoding it, which a WAV does not need.

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

pub const HEADER_BYTES: u64 = 44;
const BYTES_PER_SAMPLE: u64 = 2;

fn header(sample_rate: u32, data_bytes: u32) -> [u8; HEADER_BYTES as usize] {
    let mut h = [0u8; HEADER_BYTES as usize];
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&(36 + data_bytes).to_le_bytes());
    h[8..12].copy_from_slice(b"WAVE");
    h[12..16].copy_from_slice(b"fmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
    h[22..24].copy_from_slice(&1u16.to_le_bytes()); // mono
    h[24..28].copy_from_slice(&sample_rate.to_le_bytes());
    h[28..32].copy_from_slice(&(sample_rate * BYTES_PER_SAMPLE as u32).to_le_bytes());
    h[32..34].copy_from_slice(&(BYTES_PER_SAMPLE as u16).to_le_bytes());
    h[34..36].copy_from_slice(&16u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data_bytes.to_le_bytes());
    h
}

pub fn to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

pub struct WavFile {
    file: BufWriter<File>,
    sample_rate: u32,
    samples: u64,
}

impl WavFile {
    /// Create `path`, refusing an existing file so a planted link is never
    /// written through.
    pub fn create(path: &Path, sample_rate: u32) -> io::Result<Self> {
        let file = OpenOptions::new().write(true).create_new(true).open(path)?;
        let mut file = BufWriter::new(file);
        file.write_all(&header(sample_rate, 0))?;
        Ok(Self {
            file,
            sample_rate,
            samples: 0,
        })
    }

    pub fn write(&mut self, samples: &[f32]) -> io::Result<()> {
        for &s in samples {
            self.file.write_all(&to_i16(s).to_le_bytes())?;
        }
        self.samples += samples.len() as u64;
        Ok(())
    }

    pub fn samples(&self) -> u64 {
        self.samples
    }

    /// Hand buffered bytes to the OS, so a crash of this process (not of the
    /// machine) loses nothing already written.
    pub fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }

    /// Write the final header and make the file durable.
    /// Write the final header and hand back the file, not yet synced: the
    /// caller decides where the sync may block.
    pub fn close(self) -> io::Result<File> {
        let data_bytes = (self.samples * BYTES_PER_SAMPLE) as u32;
        let mut file = self.file.into_inner().map_err(|e| e.into_error())?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&header(self.sample_rate, data_bytes))?;
        Ok(file)
    }

    /// [`WavFile::close`], then make the file durable.
    pub fn finish(self) -> io::Result<()> {
        self.close()?.sync_all()
    }
}

/// Rewrite the header of a WAV cut short by a crash from its byte length,
/// dropping a trailing half sample. Returns the sample rate and the number
/// of whole samples it holds.
pub fn repair(path: &Path) -> io::Result<(u32, u64)> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let mut h = [0u8; HEADER_BYTES as usize];
    file.read_exact(&mut h)?;
    if &h[0..4] != b"RIFF" || &h[8..12] != b"WAVE" || &h[36..40] != b"data" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a Longhand WAV chunk",
        ));
    }
    let sample_rate = u32::from_le_bytes(h[24..28].try_into().expect("four bytes"));
    let len = file.metadata()?.len();
    let samples = len.saturating_sub(HEADER_BYTES) / BYTES_PER_SAMPLE;
    file.set_len(HEADER_BYTES + samples * BYTES_PER_SAMPLE)?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&header(sample_rate, (samples * BYTES_PER_SAMPLE) as u32))?;
    file.sync_all()?;
    Ok((sample_rate, samples))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finished_file_reads_back_through_a_standard_decoder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.wav");
        let mut wav = WavFile::create(&path, 48_000).unwrap();
        wav.write(&[0.0, 0.5, -0.5, 1.0]).unwrap();
        wav.finish().unwrap();
        let reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.spec().sample_rate, 48_000);
        assert_eq!(reader.spec().channels, 1);
        let samples: Vec<i16> = reader.into_samples().map(Result::unwrap).collect();
        assert_eq!(samples, [0, 16384, -16384, i16::MAX]);
    }

    #[test]
    fn a_crashed_file_is_repaired_from_its_length() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.part");
        let mut wav = WavFile::create(&path, 16_000).unwrap();
        wav.write(&[0.25; 100]).unwrap();
        wav.flush().unwrap();
        drop(wav); // the header still says zero samples
        // Half a sample landed before the crash.
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(&[7])
            .unwrap();

        assert_eq!(repair(&path).unwrap(), (16_000, 100));
        let reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.duration(), 100);
    }

    #[test]
    fn an_existing_file_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.wav");
        std::fs::write(&path, b"keep").unwrap();
        assert!(WavFile::create(&path, 16_000).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"keep");
    }
}
