//! A small safe wrapper of the libopus multistream calls (KTD7). A stereo
//! segment's archive uses mapping family 1 with one mono stream for each
//! channel and no coupled streams, so no channel holds audio from the other.
//! The `opus` crate has no multistream API; it links the same libopus
//! through `opusic-sys`.

use std::ffi::{CStr, c_int};
use std::ptr::NonNull;

use opusic_sys as ffi;

/// The text of a libopus error code.
fn error_text(code: c_int) -> String {
    // SAFETY: `opus_strerror` returns a pointer to a static string for
    // every code.
    let text = unsafe { CStr::from_ptr(ffi::opus_strerror(code)) };
    text.to_string_lossy().into_owned()
}

/// The channel mapping of `channels` uncoupled streams: channel N is stream N.
fn mapping(channels: u8) -> Vec<u8> {
    (0..channels).collect()
}

/// An encoder of `channels` uncoupled mono streams.
pub struct Encoder {
    state: NonNull<ffi::OpusMSEncoder>,
    channels: usize,
}

// SAFETY: the encoder state is owned by this value only, and libopus keeps
// no thread-local state.
unsafe impl Send for Encoder {}

impl Encoder {
    /// An encoder at `rate` Hz with `bitrate` bits per second for each stream.
    pub fn new(rate: u32, channels: u8, bitrate: i32) -> Result<Self, String> {
        let mapping = mapping(channels);
        let mut error = 0;
        // SAFETY: `mapping` holds `channels` entries and lives for the call;
        // libopus copies it.
        let state = unsafe {
            ffi::opus_multistream_encoder_create(
                rate as i32,
                c_int::from(channels),
                c_int::from(channels),
                0,
                mapping.as_ptr(),
                ffi::OPUS_APPLICATION_AUDIO,
                &mut error,
            )
        };
        let state = NonNull::new(state)
            .filter(|_| error == ffi::OPUS_OK)
            .ok_or_else(|| error_text(error))?;
        let encoder = Self {
            state,
            channels: usize::from(channels),
        };
        let total = bitrate * i32::from(channels);
        // SAFETY: the request takes one `opus_int32` argument.
        let code = unsafe {
            ffi::opus_multistream_encoder_ctl(
                encoder.state.as_ptr(),
                ffi::OPUS_SET_BITRATE_REQUEST,
                total,
            )
        };
        if code != ffi::OPUS_OK {
            return Err(error_text(code));
        }
        Ok(encoder)
    }

    /// The samples for each channel that the decoder output is late by: the
    /// pre-skip of the Ogg Opus header.
    pub fn lookahead(&mut self) -> Result<u16, String> {
        let mut value: i32 = 0;
        // SAFETY: the request takes one `*mut opus_int32` argument, which
        // points at `value` for the call.
        let code = unsafe {
            ffi::opus_multistream_encoder_ctl(
                self.state.as_ptr(),
                ffi::OPUS_GET_LOOKAHEAD_REQUEST,
                &mut value as *mut i32,
            )
        };
        if code != ffi::OPUS_OK {
            return Err(error_text(code));
        }
        u16::try_from(value).map_err(|_| format!("a lookahead of {value} samples"))
    }

    /// Encode one frame of interleaved samples into `out`, and return the
    /// packet length.
    pub fn encode(&mut self, pcm: &[f32], out: &mut [u8]) -> Result<usize, String> {
        if !pcm.len().is_multiple_of(self.channels) {
            return Err(format!(
                "{} samples are not whole frames of {} channels",
                pcm.len(),
                self.channels
            ));
        }
        let frame = (pcm.len() / self.channels) as c_int;
        // SAFETY: `pcm` holds `frame` samples of each channel, and `out`
        // holds `out.len()` bytes; both live for the call.
        let len = unsafe {
            ffi::opus_multistream_encode_float(
                self.state.as_ptr(),
                pcm.as_ptr(),
                frame,
                out.as_mut_ptr(),
                out.len().min(i32::MAX as usize) as i32,
            )
        };
        usize::try_from(len).map_err(|_| error_text(len))
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // SAFETY: the state came from `opus_multistream_encoder_create` and
        // is destroyed once.
        unsafe { ffi::opus_multistream_encoder_destroy(self.state.as_ptr()) }
    }
}

/// A decoder of `channels` uncoupled mono streams.
pub struct Decoder {
    state: NonNull<ffi::OpusMSDecoder>,
    channels: usize,
}

// SAFETY: as for `Encoder`.
unsafe impl Send for Decoder {}

impl Decoder {
    pub fn new(rate: u32, channels: u8) -> Result<Self, String> {
        let mapping = mapping(channels);
        let mut error = 0;
        // SAFETY: as for `Encoder::new`.
        let state = unsafe {
            ffi::opus_multistream_decoder_create(
                rate as i32,
                c_int::from(channels),
                c_int::from(channels),
                0,
                mapping.as_ptr(),
                &mut error,
            )
        };
        let state = NonNull::new(state)
            .filter(|_| error == ffi::OPUS_OK)
            .ok_or_else(|| error_text(error))?;
        Ok(Self {
            state,
            channels: usize::from(channels),
        })
    }

    /// Decode one packet into `out` as interleaved samples, and return the
    /// samples for each channel.
    pub fn decode(&mut self, packet: &[u8], out: &mut [f32]) -> Result<usize, String> {
        let frame = (out.len() / self.channels) as c_int;
        // SAFETY: `packet` holds `packet.len()` bytes and `out` holds
        // `frame` samples of each channel; both live for the call.
        let samples = unsafe {
            ffi::opus_multistream_decode_float(
                self.state.as_ptr(),
                packet.as_ptr(),
                packet.len().min(i32::MAX as usize) as i32,
                out.as_mut_ptr(),
                frame,
                0,
            )
        };
        usize::try_from(samples).map_err(|_| error_text(samples))
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: the state came from `opus_multistream_decoder_create` and
        // is destroyed once.
        unsafe { ffi::opus_multistream_decoder_destroy(self.state.as_ptr()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_uncoupled_streams_keep_each_channel_to_itself() {
        let mut encoder = Encoder::new(48_000, 2, 64_000).unwrap();
        let mut decoder = Decoder::new(48_000, 2).unwrap();
        assert!(encoder.lookahead().unwrap() > 0);
        let mut packet = vec![0u8; 4_000];
        let mut out = vec![0f32; 5_760 * 2];
        let (mut left, mut right) = (0.0f64, 0.0f64);
        for n in 0..50 {
            let frame: Vec<f32> = (0..960)
                .flat_map(|i| {
                    let t = (n * 960 + i) as f32 / 48_000.0;
                    [0.0, 0.5 * (std::f32::consts::TAU * 440.0 * t).sin()]
                })
                .collect();
            let len = encoder.encode(&frame, &mut packet).unwrap();
            let samples = decoder.decode(&packet[..len], &mut out).unwrap();
            assert_eq!(samples, 960);
            for pair in out[..samples * 2].chunks(2) {
                left += f64::from(pair[0]).powi(2);
                right += f64::from(pair[1]).powi(2);
            }
        }
        assert!(right > 0.0);
        assert!(10.0 * (left / right).max(1e-20).log10() < -40.0);
    }
}
