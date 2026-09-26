//! 16-bit PCM WAV, mono or stereo, written by hand so the header can be
//! repaired from the file's byte length after a crash (KTD4) — anarlog
//! measures a partial MP3 by decoding it, which a WAV does not need.
//!
//! A stereo file holds interleaved frames: the microphone on the left and
//! system audio on the right (R11, KTD6 of the system audio plan). The
//! header's channel count, byte rate and block align follow the channel
//! count, and a repair keeps only whole frames, so a crash in the middle of
//! a frame cannot move the channels by one sample.

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

pub const HEADER_BYTES: u64 = 44;
const BYTES_PER_SAMPLE: u64 = 2;

fn header(sample_rate: u32, channels: u16, data_bytes: u32) -> [u8; HEADER_BYTES as usize] {
    let block_align = channels * BYTES_PER_SAMPLE as u16;
    let mut h = [0u8; HEADER_BYTES as usize];
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&(36 + data_bytes).to_le_bytes());
    h[8..12].copy_from_slice(b"WAVE");
    h[12..16].copy_from_slice(b"fmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
    h[22..24].copy_from_slice(&channels.to_le_bytes());
    h[24..28].copy_from_slice(&sample_rate.to_le_bytes());
    h[28..32].copy_from_slice(&(sample_rate * u32::from(block_align)).to_le_bytes());
    h[32..34].copy_from_slice(&block_align.to_le_bytes());
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
    channels: u16,
    /// Samples of all channels.
    samples: u64,
}

impl WavFile {
    /// Create `path` for `channels` channels (1 or 2), refusing an existing
    /// file so a planted link is never written through.
    pub fn create(path: &Path, sample_rate: u32, channels: u16) -> io::Result<Self> {
        check_channels(channels)?;
        let file = OpenOptions::new().write(true).create_new(true).open(path)?;
        let mut file = BufWriter::new(file);
        file.write_all(&header(sample_rate, channels, 0))?;
        Ok(Self {
            file,
            sample_rate,
            channels,
            samples: 0,
        })
    }

    /// Write samples. A stereo file takes interleaved frames.
    pub fn write(&mut self, samples: &[f32]) -> io::Result<()> {
        for &s in samples {
            self.file.write_all(&to_i16(s).to_le_bytes())?;
        }
        self.samples += samples.len() as u64;
        Ok(())
    }

    /// Samples of all channels written so far.
    pub fn samples(&self) -> u64 {
        self.samples
    }

    /// Whole frames written so far: one sample of each channel.
    pub fn frames(&self) -> u64 {
        self.samples / u64::from(self.channels)
    }

    /// Hand buffered bytes to the OS, so a crash of this process (not of the
    /// machine) loses nothing already written.
    pub fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }

    /// Write the final header and hand back the file, not yet synced: the
    /// caller decides where the sync may block.
    pub fn close(self) -> io::Result<File> {
        let data_bytes = (self.frames() * u64::from(self.channels) * BYTES_PER_SAMPLE) as u32;
        let mut file = self.file.into_inner().map_err(|e| e.into_error())?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&header(self.sample_rate, self.channels, data_bytes))?;
        Ok(file)
    }

    /// [`WavFile::close`], then make the file durable.
    pub fn finish(self) -> io::Result<()> {
        self.close()?.sync_all()
    }
}

fn check_channels(channels: u16) -> io::Result<()> {
    if matches!(channels, 1 | 2) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("a Longhand WAV chunk has 1 or 2 channels, not {channels}"),
        ))
    }
}

/// Read and check the header of a Longhand chunk.
fn read_header(file: &mut File) -> io::Result<(u32, u16)> {
    let mut h = [0u8; HEADER_BYTES as usize];
    file.read_exact(&mut h)?;
    if &h[0..4] != b"RIFF" || &h[8..12] != b"WAVE" || &h[36..40] != b"data" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a Longhand WAV chunk",
        ));
    }
    let sample_rate = u32::from_le_bytes(h[24..28].try_into().expect("four bytes"));
    let channels = u16::from_le_bytes(h[22..24].try_into().expect("two bytes"));
    check_channels(channels)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    Ok((sample_rate, channels))
}

/// The sample rate and channel count in a chunk's header. The header is
/// correct from the file's creation, also in a `.part` that a crash left
/// open: only the lengths wait for the close (KTD6).
pub fn read_format(path: &Path) -> io::Result<(u32, u16)> {
    read_header(&mut File::open(path)?)
}

/// What [`repair`] found in a chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Repaired {
    pub sample_rate: u32,
    pub channels: u16,
    /// Whole frames: one sample of each channel.
    pub frames: u64,
}

/// Rewrite the header of a WAV cut short by a crash from its byte length,
/// dropping a trailing part frame.
pub fn repair(path: &Path) -> io::Result<Repaired> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let (sample_rate, channels) = read_header(&mut file)?;
    let frame_bytes = u64::from(channels) * BYTES_PER_SAMPLE;
    let len = file.metadata()?.len();
    let frames = len.saturating_sub(HEADER_BYTES) / frame_bytes;
    file.set_len(HEADER_BYTES + frames * frame_bytes)?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&header(
        sample_rate,
        channels,
        (frames * frame_bytes) as u32,
    ))?;
    file.sync_all()?;
    Ok(Repaired {
        sample_rate,
        channels,
        frames,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finished_file_reads_back_through_a_standard_decoder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.wav");
        let mut wav = WavFile::create(&path, 48_000, 1).unwrap();
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
        let mut wav = WavFile::create(&path, 16_000, 1).unwrap();
        wav.write(&[0.25; 100]).unwrap();
        wav.flush().unwrap();
        drop(wav); // the header still says zero samples
        // Half a sample reached the disk before the crash.
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(&[7])
            .unwrap();

        assert_eq!(
            repair(&path).unwrap(),
            Repaired {
                sample_rate: 16_000,
                channels: 1,
                frames: 100
            }
        );
        let reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.duration(), 100);
    }

    #[test]
    fn an_existing_file_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.wav");
        std::fs::write(&path, b"keep").unwrap();
        assert!(WavFile::create(&path, 16_000, 1).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"keep");
    }

    #[test]
    fn a_stereo_chunk_has_two_channels_in_its_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.wav");
        let mut wav = WavFile::create(&path, 48_000, 2).unwrap();
        // Interleaved: left, right, left, right.
        wav.write(&[0.5, -0.5, 0.25, 0.0]).unwrap();
        assert_eq!(wav.frames(), 2);
        wav.finish().unwrap();
        let reader = hound::WavReader::open(&path).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, 2);
        assert_eq!(spec.sample_rate, 48_000);
        assert_eq!(reader.duration(), 2, "two frames");
        let samples: Vec<i16> = reader.into_samples().map(Result::unwrap).collect();
        assert_eq!(samples, [16384, -16384, 8192, 0]);
        // The byte rate and the block align count both channels.
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(
            u32::from_le_bytes(bytes[28..32].try_into().unwrap()),
            192_000
        );
        assert_eq!(u16::from_le_bytes(bytes[32..34].try_into().unwrap()), 4);
        assert_eq!(read_format(&path).unwrap(), (48_000, 2));
    }

    #[test]
    fn a_truncated_stereo_chunk_is_repaired_to_whole_frames() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.part");
        let mut wav = WavFile::create(&path, 16_000, 2).unwrap();
        let frames: Vec<f32> = (0..200)
            .map(|i| if i % 2 == 0 { 0.25 } else { -0.25 })
            .collect();
        wav.write(&frames).unwrap();
        wav.flush().unwrap();
        drop(wav);
        // The left sample of the next frame and half of its right sample
        // reached the disk before the crash: three bytes past the last
        // whole frame.
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(&[1, 2, 3])
            .unwrap();

        assert_eq!(
            repair(&path).unwrap(),
            Repaired {
                sample_rate: 16_000,
                channels: 2,
                frames: 100,
            }
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            HEADER_BYTES + 100 * 4
        );
        let reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.spec().channels, 2);
        assert_eq!(reader.duration(), 100);
        let samples: Vec<i16> = reader.into_samples().map(Result::unwrap).collect();
        // Left and right stay in their places: no frame moves by a sample.
        assert!(samples.chunks(2).all(|f| f[0] > 0 && f[1] < 0));
    }
}
