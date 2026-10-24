//! Decode an audio file to mono samples (KTD4).

use std::fs::File;
use std::path::Path;

use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// What a decode produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeInfo {
    /// The sample rate of the file and of the samples.
    pub sample_rate: u32,
    /// The number of mono samples given to the caller.
    pub frames: u64,
}

/// Why a file did not decode.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    /// The file cannot be read.
    #[error("cannot read the file: {0}")]
    Io(#[from] std::io::Error),
    /// The file is not audio in an accepted format, or its audio is damaged.
    #[error("cannot decode the file: {0}")]
    Format(String),
    /// The caller stopped the decode ([`decode_mono_until`]).
    #[error("the decode was stopped")]
    Stopped,
}

/// Decode `path` and give its audio to `on_block` as mono samples at the
/// file's rate. A file with two or more channels gives the average of its
/// channels, because a 2-channel segment means microphone plus app audio in
/// processing (KTD4).
///
/// A packet that does not decode is skipped, as a player does. A file that
/// gives no audio at all is a [`DecodeError::Format`].
pub fn decode_mono(
    path: &Path,
    mut on_block: impl FnMut(&[f32]),
) -> Result<DecodeInfo, DecodeError> {
    decode_mono_until(path, |_, block| {
        on_block(block);
        true
    })
}

/// [`decode_mono`] that gives the sample rate with each block, and stops
/// with [`DecodeError::Stopped`] when `on_block` returns false. The import
/// worker needs the rate before it writes the first block, and it stops a
/// conversion when the user deletes the note.
pub fn decode_mono_until(
    path: &Path,
    mut on_block: impl FnMut(u32, &[f32]) -> bool,
) -> Result<DecodeInfo, DecodeError> {
    let file = File::open(path)?;
    let source = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(extension);
    }
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            source,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(format_error)?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| DecodeError::Format("the file has no audio track".into()))?;
    let track_id = track.id;
    let Some(CodecParameters::Audio(params)) = track.codec_params.clone() else {
        return Err(DecodeError::Format(
            "the audio track has no codec parameters".into(),
        ));
    };
    let sample_rate = params
        .sample_rate
        .ok_or_else(|| DecodeError::Format("the audio track has no sample rate".into()))?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(format_error)?;

    let mut interleaved: Vec<f32> = Vec::new();
    let mut mono: Vec<f32> = Vec::new();
    let mut frames = 0u64;
    let mut skipped = 0u64;
    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(e) => return Err(format_error(e)),
        };
        if packet.track_id != track_id {
            continue;
        }
        let buffer = match decoder.decode(&packet) {
            Ok(buffer) => buffer,
            Err(SymphoniaError::DecodeError(_)) => {
                skipped += 1;
                continue;
            }
            Err(e) => return Err(format_error(e)),
        };
        let channels = buffer.num_planes().max(1);
        buffer.copy_to_vec_interleaved(&mut interleaved);
        mono.clear();
        mono.extend(
            interleaved
                .chunks_exact(channels)
                .map(|frame| frame.iter().sum::<f32>() / channels as f32),
        );
        frames += mono.len() as u64;
        if !on_block(sample_rate, &mono) {
            return Err(DecodeError::Stopped);
        }
    }
    if skipped > 0 {
        log::warn!("audio import: skipped {skipped} packets that did not decode");
    }
    if frames == 0 {
        return Err(DecodeError::Format("the file gave no audio".into()));
    }
    Ok(DecodeInfo {
        sample_rate,
        frames,
    })
}

fn format_error(e: SymphoniaError) -> DecodeError {
    match e {
        SymphoniaError::IoError(e) if e.kind() != std::io::ErrorKind::UnexpectedEof => {
            DecodeError::Io(e)
        }
        other => DecodeError::Format(other.to_string()),
    }
}
