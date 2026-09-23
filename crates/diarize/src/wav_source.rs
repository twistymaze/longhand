//! A 16 kHz mono f32 WAV read in place, so a four-hour segment is never
//! resident (R7). Transcribed from anarlog's `WavAudioSource`; see
//! PROVENANCE.md.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use pyannote_local::{AudioSource, SAMPLE_RATE};

use crate::DiarizeError;

pub struct WavAudioSource {
    reader: hound::WavReader<BufReader<File>>,
    len: usize,
}

impl WavAudioSource {
    pub fn open(path: &Path) -> Result<Self, DiarizeError> {
        let reader = hound::WavReader::open(path)
            .map_err(|e| DiarizeError::Format(format!("{}: {e}", path.display())))?;
        let spec = reader.spec();
        if spec.channels != 1
            || spec.sample_rate != SAMPLE_RATE
            || spec.sample_format != hound::SampleFormat::Float
        {
            return Err(DiarizeError::Format(format!(
                "{} must be 16 kHz mono float, got {} Hz x{} {:?}",
                path.display(),
                spec.sample_rate,
                spec.channels,
                spec.sample_format
            )));
        }
        let len = reader.len() as usize;
        Ok(Self { reader, len })
    }
}

impl AudioSource for WavAudioSource {
    fn sample_count(&mut self) -> Result<usize, pyannote_local::Error> {
        Ok(self.len)
    }

    fn read(&mut self, start: usize, out: &mut [f32]) -> Result<(), pyannote_local::Error> {
        out.fill(0.0);
        if start >= self.len {
            return Ok(());
        }
        let start_u32 =
            u32::try_from(start).map_err(|e| pyannote_local::Error::AudioRead(e.to_string()))?;
        self.reader
            .seek(start_u32)
            .map_err(|e| pyannote_local::Error::AudioRead(e.to_string()))?;
        let available = out.len().min(self.len - start);
        for (slot, sample) in out[..available]
            .iter_mut()
            .zip(self.reader.samples::<f32>())
        {
            *slot = sample.map_err(|e| pyannote_local::Error::AudioRead(e.to_string()))?;
        }
        Ok(())
    }
}
