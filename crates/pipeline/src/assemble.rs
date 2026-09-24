//! The assemble stage (KTD4, KD16): a segment's overlapping native-rate
//! chunks become one continuous stream, written twice — the full-band Opus
//! archive that lands with the session, and the 16 kHz mono f32 tap that
//! recognition and diarization read. Audio streams through in blocks; no
//! segment is ever held in memory (R7).

use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use ogg::writing::{PacketWriteEndInfo, PacketWriter};
use serde::{Deserialize, Serialize};
use session_core::{Segment, Stage};

use crate::resample::StreamResampler;
use crate::{Control, RECOVERY_DIR, SharedStore, StageError, checkpoint, proc_file};

pub const PROC_RATE: u32 = 16_000;
/// Opus always runs at 48 kHz internally; other input rates are resampled.
const OPUS_RATE: u32 = 48_000;
const OPUS_FRAME: usize = 960; // 20 ms
const OPUS_BITRATE: i32 = 64_000;
/// Packets per Ogg page: one second of audio.
const PACKETS_PER_PAGE: u64 = 50;
const BLOCK: usize = 4_096;

/// The assemble checkpoint for one segment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssembledSegment {
    pub segment: u32,
    pub native_rate: u32,
    /// Samples in the trimmed native-rate stream.
    pub native_samples: u64,
    /// Samples in the 16 kHz processing file.
    pub proc_samples: u64,
}

/// Assemble every segment of the session that is not yet assembled. `home`
/// is the folder a resumed session completed into (R6d): a segment whose
/// archive is there is already assembled, and its processing file is decoded
/// back from that archive.
/// Refuse to start when the working location cannot hold what assembling
/// the remaining segments writes (R7b): better named now, with the session
/// whole, than as a write failure part-way through.
fn check_space(
    store: &SharedStore,
    session_id: &str,
    session_dir: &Path,
    segments: &[Segment],
) -> Result<(), StageError> {
    let mut remaining_ms = 0;
    for segment in segments {
        let key = checkpoint::segment_key(segment.index);
        if checkpoint::get::<AssembledSegment>(store, session_id, Stage::Assemble, &key)?.is_none()
        {
            remaining_ms += segment.duration_ms;
        }
    }
    let needed = recorder::storage::processing_demand(remaining_ms.div_ceil(1_000));
    let available = recorder::storage::available_space(session_dir);
    recorder::storage::check_headroom(available, needed, recorder::storage::DISK_RESERVE_BYTES)
        .map_err(|free| {
            StageError::Failed(format!(
                "processing needs about {} free on the disk holding Longhand's working audio, and {} is free; free some space, then retry",
                gigabytes(needed + recorder::storage::DISK_RESERVE_BYTES),
                gigabytes(free)
            ))
        })
}

fn gigabytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

pub fn run(
    store: &SharedStore,
    session_id: &str,
    session_dir: &Path,
    segments: &[Segment],
    home: Option<&Path>,
    control: &dyn Control,
) -> Result<Vec<AssembledSegment>, StageError> {
    let recovery = session_dir.join(RECOVERY_DIR);
    // A crash while capturing leaves a `.part` open; close it from its length.
    recorder::recover_partial_chunks(&recovery)?;
    check_space(store, session_id, session_dir, segments)?;
    let mut out = Vec::new();
    for segment in segments {
        let key = checkpoint::segment_key(segment.index);
        if let Some(done) =
            checkpoint::get::<AssembledSegment>(store, session_id, Stage::Assemble, &key)?
            && session_dir.join(&segment.audio_file).exists()
            && session_dir.join(proc_file(&segment.audio_file)).exists()
        {
            out.push(done);
            continue;
        }
        if let Some(archive) = home
            .map(|home| home.join(&segment.audio_file))
            .filter(|archive| archive.exists())
        {
            control.proceed()?;
            let proc_samples =
                decode_archive(&archive, &session_dir.join(proc_file(&segment.audio_file)))?;
            let done = AssembledSegment {
                segment: segment.index,
                native_rate: OPUS_RATE,
                native_samples: proc_samples * (OPUS_RATE / PROC_RATE) as u64,
                proc_samples,
            };
            checkpoint::put(store, session_id, Stage::Assemble, &key, &done)?;
            log::info!(
                "decoded segment {} of {session_id} from {}",
                segment.index,
                archive.display()
            );
            out.push(done);
            continue;
        }
        if let Some(home) = home
            && !recorder::list_chunks(&recovery)?
                .iter()
                .any(|c| c.segment == segment.index)
        {
            return Err(StageError::Failed(format!(
                "segment {}'s audio is in {}, which is missing or incomplete; put the folder back and retry",
                segment.index + 1,
                home.display()
            )));
        }
        control.proceed()?;
        let done = assemble_segment(&recovery, session_dir, segment, control)?;
        checkpoint::put(store, session_id, Stage::Assemble, &key, &done)?;
        log::info!(
            "assembled segment {} of {session_id}: {} samples at {} Hz",
            segment.index,
            done.native_samples,
            done.native_rate
        );
        out.push(done);
    }
    checkpoint::mark_complete(store, session_id, Stage::Assemble, segments.len())?;
    Ok(out)
}

pub fn assemble_segment(
    recovery: &Path,
    session_dir: &Path,
    segment: &Segment,
    control: &dyn Control,
) -> Result<AssembledSegment, StageError> {
    let chunks: Vec<_> = recorder::list_chunks(recovery)?
        .into_iter()
        .filter(|c| c.segment == segment.index)
        .collect();
    let Some(first) = chunks.first() else {
        return Err(StageError::Failed(format!(
            "segment {} has no audio chunks",
            segment.index
        )));
    };
    let rate = hound::WavReader::open(&first.path)?.spec().sample_rate;

    let archive = session_dir.join(&segment.audio_file);
    let processing = session_dir.join(proc_file(&segment.audio_file));
    let mut opus = OpusWriter::create(&temp_path(&archive), rate)?;
    let mut proc = ProcWriter::create(&temp_path(&processing))?;
    let mut to_opus = StreamResampler::new(rate, OPUS_RATE);
    let mut to_proc = StreamResampler::new(rate, PROC_RATE);

    let mut native_samples = 0u64;
    let mut failure: Option<StageError> = None;
    let mut feed = |block: &[f32], native_samples: &mut u64| {
        *native_samples += block.len() as u64;
        to_opus.push(block, &mut |s| {
            if failure.is_none()
                && let Err(e) = opus.push(s)
            {
                failure = Some(e);
            }
        });
        to_proc.push(block, &mut |s| {
            if failure.is_none()
                && let Err(e) = proc.push(s)
            {
                failure = Some(e);
            }
        });
    };

    let mut expected_ms = first.start_ms;
    let mut block = Vec::with_capacity(BLOCK);
    for chunk in &chunks {
        control.proceed()?;
        let mut reader = hound::WavReader::open(&chunk.path)?;
        let spec = reader.spec();
        if spec.sample_rate != rate || spec.channels != 1 || spec.bits_per_sample != 16 {
            return Err(StageError::Failed(format!(
                "chunk {} is not 16-bit mono at {rate} Hz",
                chunk.path.display()
            )));
        }
        // A missing chunk leaves a hole; silence keeps what follows on the timeline.
        if chunk.start_ms > expected_ms {
            let gap = ms_to_samples(chunk.start_ms - expected_ms, rate);
            log::warn!(
                "segment {}: {} ms missing before {}; filled with silence",
                segment.index,
                chunk.start_ms - expected_ms,
                chunk.path.display()
            );
            for part in vec![0.0f32; gap as usize].chunks(BLOCK) {
                feed(part, &mut native_samples);
            }
        }
        // The lookback repeats the previous chunk's tail; skip it.
        let lookback = ms_to_samples(chunk.start_ms.saturating_sub(chunk.audio_start_ms), rate);
        let overlap = ms_to_samples(expected_ms.saturating_sub(chunk.start_ms), rate);
        let skip = lookback + overlap;
        for (i, sample) in reader.samples::<i16>().enumerate() {
            if (i as u64) < skip {
                continue;
            }
            block.push(sample? as f32 / 32_768.0);
            if block.len() == BLOCK {
                feed(&block, &mut native_samples);
                block.clear();
            }
        }
        expected_ms = expected_ms.max(chunk.end_ms);
    }
    if !block.is_empty() {
        feed(&block, &mut native_samples);
    }
    to_opus.finish(&mut |s| {
        if failure.is_none()
            && let Err(e) = opus.push(s)
        {
            failure = Some(e);
        }
    });
    to_proc.finish(&mut |s| {
        if failure.is_none()
            && let Err(e) = proc.push(s)
        {
            failure = Some(e);
        }
    });
    if let Some(e) = failure {
        return Err(e);
    }
    opus.finish()?;
    let proc_samples = proc.finish()?;
    commit(&temp_path(&archive), &archive)?;
    commit(&temp_path(&processing), &processing)?;

    Ok(AssembledSegment {
        segment: segment.index,
        native_rate: rate,
        native_samples,
        proc_samples,
    })
}

/// Decode an archive this stage wrote back into a 16 kHz processing file,
/// trimmed as a player would (RFC 7845: pre-skip at the start, the last
/// granule at the end). Returns the processing samples written.
pub fn decode_archive(archive: &Path, processing: &Path) -> Result<u64, StageError> {
    let bad = |why: &str| StageError::Failed(format!("{}: {why}", archive.display()));
    let mut packets = ogg::PacketReader::new(BufReader::new(File::open(archive)?));
    let head = packets
        .read_packet()
        .map_err(|e| bad(&e.to_string()))?
        .ok_or_else(|| bad("empty"))?;
    if head.data.len() < 19 || &head.data[..8] != b"OpusHead" || head.data[9] != 1 {
        return Err(bad("not a mono Opus stream"));
    }
    let pre_skip = u16::from_le_bytes([head.data[10], head.data[11]]) as u64;
    packets
        .read_packet()
        .map_err(|e| bad(&e.to_string()))?
        .ok_or_else(|| bad("no tags"))?;

    let mut decoder = opus::Decoder::new(OPUS_RATE, opus::Channels::Mono)
        .map_err(|e| bad(&format!("opus decoder: {e}")))?;
    let mut to_proc = StreamResampler::new(OPUS_RATE, PROC_RATE);
    let mut proc = ProcWriter::create(&temp_path(processing))?;
    let mut failure: Option<StageError> = None;
    let mut buf = vec![0f32; 5_760];
    // 48 kHz samples decoded so far, pre-skip included.
    let mut decoded = 0u64;
    while let Some(packet) = packets.read_packet().map_err(|e| bad(&e.to_string()))? {
        let n = decoder
            .decode_float(&packet.data, &mut buf, false)
            .map_err(|e| bad(&format!("opus decoder: {e}")))? as u64;
        let end = if packet.last_in_stream() {
            (decoded + n).min(packet.absgp_page())
        } else {
            decoded + n
        };
        let from = pre_skip.max(decoded).min(end);
        let pcm = &buf[(from - decoded) as usize..(end - decoded) as usize];
        decoded += n;
        to_proc.push(pcm, &mut |s| {
            if failure.is_none()
                && let Err(e) = proc.push(s)
            {
                failure = Some(e);
            }
        });
    }
    to_proc.finish(&mut |s| {
        if failure.is_none()
            && let Err(e) = proc.push(s)
        {
            failure = Some(e);
        }
    });
    if let Some(e) = failure {
        return Err(e);
    }
    let samples = proc.finish()?;
    commit(&temp_path(processing), processing)?;
    Ok(samples)
}

fn ms_to_samples(ms: u64, rate: u32) -> u64 {
    (ms * rate as u64 + 500) / 1000
}

fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

/// Create `path` fresh, replacing whatever an interrupted run left there.
fn create_fresh(path: &Path) -> std::io::Result<File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    OpenOptions::new().write(true).create_new(true).open(path)
}

/// Make a finished file durable under its real name.
fn commit(temp: &Path, path: &Path) -> std::io::Result<()> {
    crate::sync_file(temp)?;
    std::fs::rename(temp, path)?;
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

/// 16 kHz mono IEEE-float WAV.
struct ProcWriter {
    writer: hound::WavWriter<BufWriter<File>>,
    samples: u64,
}

impl ProcWriter {
    fn create(path: &Path) -> Result<Self, StageError> {
        let file = create_fresh(path)?;
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: PROC_RATE,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        Ok(Self {
            writer: hound::WavWriter::new(BufWriter::new(file), spec)?,
            samples: 0,
        })
    }

    fn push(&mut self, samples: &[f32]) -> Result<(), StageError> {
        for &s in samples {
            self.writer.write_sample(s)?;
        }
        self.samples += samples.len() as u64;
        Ok(())
    }

    fn finish(self) -> Result<u64, StageError> {
        self.writer.finalize()?;
        Ok(self.samples)
    }
}

/// Mono Opus in Ogg (RFC 7845), streamed a 20 ms frame at a time.
struct OpusWriter {
    encoder: opus::Encoder,
    packets: PacketWriter<'static, BufWriter<File>>,
    serial: u32,
    pre_skip: u64,
    frame: Vec<f32>,
    /// Real (unpadded) 48 kHz samples encoded.
    samples: u64,
    /// The newest packet, held back so the last one can close the stream.
    held: Option<Vec<u8>>,
    packets_written: u64,
    buf: Vec<u8>,
}

impl OpusWriter {
    fn create(path: &Path, input_rate: u32) -> Result<Self, StageError> {
        let opus_error = |e: opus::Error| StageError::Failed(format!("opus encoder: {e}"));
        let mut encoder =
            opus::Encoder::new(OPUS_RATE, opus::Channels::Mono, opus::Application::Audio)
                .map_err(opus_error)?;
        encoder
            .set_bitrate(opus::Bitrate::Bits(OPUS_BITRATE))
            .map_err(opus_error)?;
        let pre_skip = encoder.get_lookahead().map_err(opus_error)?.max(0) as u64;
        let mut packets = PacketWriter::new(BufWriter::new(create_fresh(path)?));
        // A fixed serial: one logical stream per file, and assembly stays
        // byte-for-byte reproducible.
        let serial = 0x4c48_0001;

        let mut head = Vec::with_capacity(19);
        head.extend_from_slice(b"OpusHead");
        head.push(1); // version
        head.push(1); // channels
        head.extend_from_slice(&(pre_skip as u16).to_le_bytes());
        head.extend_from_slice(&input_rate.to_le_bytes());
        head.extend_from_slice(&0i16.to_le_bytes()); // output gain
        head.push(0); // mapping family: mono/stereo
        packets.write_packet(head, serial, PacketWriteEndInfo::EndPage, 0)?;

        let vendor = b"Longhand";
        let mut tags = Vec::new();
        tags.extend_from_slice(b"OpusTags");
        tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        tags.extend_from_slice(vendor);
        tags.extend_from_slice(&0u32.to_le_bytes()); // no comments
        packets.write_packet(tags, serial, PacketWriteEndInfo::EndPage, 0)?;

        Ok(Self {
            encoder,
            packets,
            serial,
            pre_skip,
            frame: Vec::with_capacity(OPUS_FRAME),
            samples: 0,
            held: None,
            packets_written: 0,
            buf: vec![0; 4_000],
        })
    }

    fn push(&mut self, mut samples: &[f32]) -> Result<(), StageError> {
        while !samples.is_empty() {
            let take = (OPUS_FRAME - self.frame.len()).min(samples.len());
            self.frame.extend_from_slice(&samples[..take]);
            samples = &samples[take..];
            if self.frame.len() == OPUS_FRAME {
                self.samples += OPUS_FRAME as u64;
                self.encode_frame()?;
            }
        }
        Ok(())
    }

    fn encode_frame(&mut self) -> Result<(), StageError> {
        let len = self
            .encoder
            .encode_float(&self.frame, &mut self.buf)
            .map_err(|e| StageError::Failed(format!("opus encoder: {e}")))?;
        self.frame.clear();
        if let Some(previous) = self.held.replace(self.buf[..len].to_vec()) {
            self.write(previous, false)?;
        }
        Ok(())
    }

    fn write(&mut self, packet: Vec<u8>, last: bool) -> Result<(), StageError> {
        self.packets_written += 1;
        let end = if last {
            PacketWriteEndInfo::EndStream
        } else if self.packets_written.is_multiple_of(PACKETS_PER_PAGE) {
            PacketWriteEndInfo::EndPage
        } else {
            PacketWriteEndInfo::NormalPacket
        };
        // A page's granule is the 48 kHz samples decoded through its last
        // packet, pre-skip included; the final one stops at the real end so
        // a decoder trims the padding (RFC 7845 §4).
        let granule = if last {
            self.pre_skip + self.samples
        } else {
            self.packets_written * OPUS_FRAME as u64
        };
        self.packets
            .write_packet(packet, self.serial, end, granule)?;
        Ok(())
    }

    fn finish(mut self) -> Result<(), StageError> {
        // The decoder's output runs `pre_skip` samples behind the input, so
        // frames of silence past the end carry the last real samples out; the
        // final granule trims them again (RFC 7845 §4).
        let real = self.samples + self.frame.len() as u64;
        let needed = (real + self.pre_skip).div_ceil(OPUS_FRAME as u64);
        while self.packets_written + u64::from(self.held.is_some()) < needed {
            self.frame.resize(OPUS_FRAME, 0.0);
            self.encode_frame()?;
        }
        self.samples = real;
        match self.held.take() {
            Some(last) => self.write(last, true)?,
            None => {
                // No audio at all: close the stream with one frame of silence.
                self.frame.resize(OPUS_FRAME, 0.0);
                self.encode_frame()?;
                let last = self.held.take().expect("just encoded");
                self.write(last, true)?;
            }
        }
        self.packets.inner_mut().flush()?;
        Ok(())
    }
}

/// Read the 16 kHz processing file of a segment.
pub fn open_proc(
    session_dir: &Path,
    segment: &Segment,
) -> Result<hound::WavReader<BufReader<File>>, StageError> {
    let path = session_dir.join(proc_file(&segment.audio_file));
    let reader = hound::WavReader::open(&path)?;
    let spec = reader.spec();
    if spec.sample_rate != PROC_RATE
        || spec.channels != 1
        || spec.sample_format != hound::SampleFormat::Float
    {
        return Err(StageError::Failed(format!(
            "{} is not 16 kHz mono float",
            path.display()
        )));
    }
    Ok(reader)
}
