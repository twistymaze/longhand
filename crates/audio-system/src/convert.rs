//! Interleaved PCM bytes to mono frames at one output rate (U4 step 4).
//!
//! The Windows backend reads loopback packets in the format of the output
//! device, for example a 5.1 device. The frames have one channel and one
//! rate.

use crate::rate::RateFollower;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleKind {
    /// 32-bit float, little-endian.
    F32,
    /// 16-bit signed integer, little-endian.
    I16,
    /// 32-bit signed integer, little-endian.
    I32,
}

impl SampleKind {
    pub fn bytes(self) -> usize {
        match self {
            SampleKind::I16 => 2,
            SampleKind::F32 | SampleKind::I32 => 4,
        }
    }

    fn read(self, bytes: &[u8]) -> f32 {
        match self {
            SampleKind::F32 => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            SampleKind::I16 => f32::from(i16::from_le_bytes([bytes[0], bytes[1]])) / 32_768.0,
            SampleKind::I32 => {
                i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f32
                    / 2_147_483_648.0
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcmFormat {
    pub channels: usize,
    pub kind: SampleKind,
    pub sample_rate: u32,
}

/// Append the mono mix of interleaved PCM `data` to `out`: the mean of the
/// channels of each sample frame. An incomplete last sample frame is left
/// out.
pub fn downmix(data: &[u8], channels: usize, kind: SampleKind, out: &mut Vec<f32>) {
    if channels == 0 {
        return;
    }
    let width = kind.bytes();
    let frame = channels * width;
    let scale = 1.0 / channels as f32;
    out.extend(data.chunks_exact(frame).map(|samples| {
        samples
            .chunks_exact(width)
            .map(|bytes| kind.read(bytes))
            .sum::<f32>()
            * scale
    }));
}

/// Converts loopback packets to mono frames of a fixed size at `out_hz`.
pub struct Converter {
    format: PcmFormat,
    rate: RateFollower,
    mono: Vec<f32>,
}

impl Converter {
    pub fn new(format: PcmFormat, out_hz: u32, frame_samples: usize) -> Self {
        Self {
            format,
            rate: RateFollower::new(format.sample_rate, out_hz, frame_samples),
            mono: Vec::new(),
        }
    }

    /// Convert one packet and give each complete frame to `emit`.
    pub fn push(&mut self, data: &[u8], emit: impl FnMut(&[f32])) {
        self.mono.clear();
        downmix(data, self.format.channels, self.format.kind, &mut self.mono);
        self.rate.push(&self.mono, emit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f32_bytes(samples: &[f32]) -> Vec<u8> {
        samples.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    #[test]
    fn six_channels_become_their_mean() {
        let data = f32_bytes(&[0.6, 0.0, 0.0, 0.0, 0.0, 0.0, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1]);
        let mut out = Vec::new();
        downmix(&data, 6, SampleKind::F32, &mut out);
        assert_eq!(out.len(), 2);
        assert!((out[0] - 0.1).abs() < 1e-6);
        assert!((out[1] - 0.1).abs() < 1e-6);
    }

    #[test]
    fn integer_samples_scale_to_one() {
        let data: Vec<u8> = [16_384_i16, -16_384]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let mut out = Vec::new();
        downmix(&data, 1, SampleKind::I16, &mut out);
        assert_eq!(out, [0.5, -0.5]);

        let data: Vec<u8> = [1_073_741_824_i32]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        out.clear();
        downmix(&data, 1, SampleKind::I32, &mut out);
        assert_eq!(out, [0.5]);
    }

    #[test]
    fn an_incomplete_sample_frame_is_left_out() {
        let data = f32_bytes(&[0.2, 0.2, 0.2]);
        let mut out = Vec::new();
        downmix(&data, 2, SampleKind::F32, &mut out);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn six_channels_become_mono_frames_at_the_output_rate() {
        let six = PcmFormat {
            channels: 6,
            kind: SampleKind::F32,
            sample_rate: 44_100,
        };
        let mut converter = Converter::new(six, 48_000, 960);
        let mut frames = Vec::new();

        let packet: Vec<f32> = std::iter::repeat_n([0.3_f32, 0.3, 0.3, 0.3, 0.3, 0.3], 44_100)
            .flatten()
            .collect();
        converter.push(&f32_bytes(&packet), |f| frames.push(f.to_vec()));
        assert!(
            (45..=50).contains(&frames.len()),
            "one second at 48 kHz, got {} frames",
            frames.len()
        );
        assert!(frames.iter().all(|f| f.len() == 960), "every frame is mono");
        let settled = &frames[frames.len() - 1];
        assert!(
            settled.iter().all(|s| (s - 0.3).abs() < 0.02),
            "the level passes through"
        );
    }
}
