//! Streaming sample-rate conversion for the assemble stage.
//!
//! Reshaped from Handy's `FrameResampler` (via `crates/audio-cpal`); see
//! PROVENANCE.md. The capture path wants fixed frames and tolerates the
//! filter's latency; the files written after stop sit on the session
//! timeline, so here the leading filter delay is dropped and the output is
//! exactly `input × out / in` samples long.

use rubato::{FftFixedIn, Resampler};

const CHUNK: usize = 1024;

/// Round to the nearest output sample, including the final partial sample.
pub(crate) fn output_sample_count(samples: u64, in_hz: u64, out_hz: u64) -> u64 {
    let count =
        (u128::from(samples) * u128::from(out_hz) + u128::from(in_hz / 2)) / u128::from(in_hz);
    u64::try_from(count).expect("resampled duration fits the sample timeline")
}

pub struct StreamResampler {
    inner: Option<FftFixedIn<f32>>,
    in_hz: u64,
    out_hz: u64,
    in_buf: Vec<f32>,
    /// Leading delay-line samples still to discard.
    skip: usize,
    in_count: u64,
    out_count: u64,
    /// The output samples of the pre-roll, which [`Self::emit_trimmed`]
    /// drops.
    pre_out: u64,
}

/// The input samples of one FFT block of the resampler from `in_hz` to
/// `out_hz`, as rubato's `FftFixedIn` computes it for [`CHUNK`]. The
/// output of a block depends only on the input of that block and of the
/// block before it. So two resamplers that start on the same block grid
/// give the same samples after their first block. 1 for equal rates.
pub(crate) fn block_input(in_hz: u32, out_hz: u32) -> usize {
    if in_hz == out_hz {
        return 1;
    }
    let min_chunk_in = (in_hz / gcd(in_hz, out_hz)) as usize;
    CHUNK.div_ceil(min_chunk_in) * min_chunk_in
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

impl StreamResampler {
    pub fn new(in_hz: u32, out_hz: u32) -> Self {
        let inner = (in_hz != out_hz).then(|| {
            FftFixedIn::<f32>::new(in_hz as usize, out_hz as usize, CHUNK, 1, 1)
                .expect("resampler accepts any pair of positive rates")
        });
        let skip = inner.as_ref().map_or(0, |r| r.output_delay());
        Self {
            inner,
            in_hz: in_hz as u64,
            out_hz: out_hz as u64,
            in_buf: Vec::with_capacity(CHUNK),
            skip,
            in_count: 0,
            out_count: 0,
            pre_out: 0,
        }
    }

    /// Give the resampler input that comes before its stream, and drop the
    /// output of that input. Call it before the first [`Self::push`]. A
    /// part of a cut segment uses the lookback of its first chunk this way
    /// (KTD16 of the forgotten-recording plan): when the pre-roll starts on
    /// the block grid of the original segment and holds at least one
    /// block, the part gets the same samples as the original segment.
    pub fn pre_roll(&mut self, src: &[f32]) {
        self.push(src, &mut |_| {});
        self.pre_out = self.expected();
    }

    fn expected(&self) -> u64 {
        output_sample_count(self.in_count, self.in_hz, self.out_hz)
    }

    pub fn push(&mut self, mut src: &[f32], emit: &mut impl FnMut(&[f32])) {
        self.in_count += src.len() as u64;
        if self.inner.is_none() {
            self.out_count += src.len() as u64;
            emit(src);
            return;
        }
        while !src.is_empty() {
            let take = (CHUNK - self.in_buf.len()).min(src.len());
            self.in_buf.extend_from_slice(&src[..take]);
            src = &src[take..];
            if self.in_buf.len() == CHUNK {
                let out = self
                    .inner
                    .as_mut()
                    .expect("checked above")
                    .process(&[&self.in_buf[..]], None)
                    .expect("a full chunk is the size the resampler was built for");
                self.in_buf.clear();
                self.emit_trimmed(&out[0], emit);
            }
        }
    }

    /// Flush the delay line so every input sample has produced its output.
    pub fn finish(&mut self, emit: &mut impl FnMut(&[f32])) {
        if self.inner.is_none() {
            return;
        }
        if !self.in_buf.is_empty() {
            let tail = std::mem::take(&mut self.in_buf);
            if let Ok(out) = self
                .inner
                .as_mut()
                .expect("checked above")
                .process_partial(Some(&[&tail[..]]), None)
            {
                self.emit_trimmed(&out[0], emit);
            }
        }
        let mut rounds = 0;
        while self.out_count < self.expected() && rounds < 16 {
            rounds += 1;
            match self
                .inner
                .as_mut()
                .expect("checked above")
                .process_partial::<&[f32]>(None, None)
            {
                Ok(out) => self.emit_trimmed(&out[0], emit),
                Err(_) => break,
            }
        }
        // The filter is flushed; any shortfall is a rounding sample or two.
        let short = self.expected().saturating_sub(self.out_count) as usize;
        if short > 0 {
            self.out_count += short as u64;
            emit(&vec![0.0; short]);
        }
    }

    fn emit_trimmed(&mut self, mut data: &[f32], emit: &mut impl FnMut(&[f32])) {
        let drop = self.skip.min(data.len());
        self.skip -= drop;
        data = &data[drop..];
        let room = self.expected().saturating_sub(self.out_count) as usize;
        let mut data = &data[..data.len().min(room)];
        let pre = (self.pre_out.saturating_sub(self.out_count) as usize).min(data.len());
        self.out_count += pre as u64;
        data = &data[pre..];
        if !data.is_empty() {
            self.out_count += data.len() as u64;
            emit(data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(in_hz: u32, out_hz: u32, input: &[f32], block: usize) -> Vec<f32> {
        let mut r = StreamResampler::new(in_hz, out_hz);
        let mut out = Vec::new();
        for chunk in input.chunks(block) {
            r.push(chunk, &mut |s| out.extend_from_slice(s));
        }
        r.finish(&mut |s| out.extend_from_slice(s));
        out
    }

    #[test]
    fn output_length_is_exactly_the_rate_ratio() {
        for (in_hz, out_hz, n) in [
            (48_000, 16_000, 48_000 * 3 + 17),
            (44_100, 16_000, 44_100 * 2 + 5),
            (44_100, 48_000, 9_999),
        ] {
            let input = vec![0.1f32; n];
            let out = run(in_hz, out_hz, &input, 777);
            let expected = (n as u64 * out_hz as u64 + in_hz as u64 / 2) / in_hz as u64;
            assert_eq!(out.len() as u64, expected, "{in_hz} -> {out_hz}");
        }
    }

    #[test]
    fn a_click_stays_where_it_was_on_the_timeline() {
        // An impulse at 1.5 s must come out at 1.5 s, not a filter delay late.
        let mut input = vec![0.0f32; 48_000 * 3];
        input[72_000] = 1.0;
        let out = run(48_000, 16_000, &input, 4_096);
        let peak = out
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .unwrap()
            .0;
        assert!((peak as i64 - 24_000).abs() <= 2, "peak at {peak}");
    }

    /// Resample `input[part..]` with a pre-roll from the lookback before
    /// `part` that starts on the block grid of `input`.
    fn run_part(in_hz: u32, input: &[f32], part: usize, lookback: usize) -> Vec<f32> {
        let block = block_input(in_hz, 16_000);
        let pre = (lookback - part % block) / block * block + part % block;
        let mut r = StreamResampler::new(in_hz, 16_000);
        r.pre_roll(&input[part - pre..part]);
        let mut out = Vec::new();
        for chunk in input[part..].chunks(4_096) {
            r.push(chunk, &mut |s| out.extend_from_slice(s));
        }
        r.finish(&mut |s| out.extend_from_slice(s));
        out
    }

    #[test]
    fn a_part_with_a_pre_roll_on_the_block_grid_has_the_samples_of_the_whole() {
        for in_hz in [48_000u32, 44_100] {
            let input: Vec<f32> = (0..in_hz as usize * 9)
                .map(|i| ((i * 7_919 % 2_003) as f32 / 2_003.0 - 0.5) * 0.8)
                .collect();
            let whole = run(in_hz, 16_000, &input, 4_096);
            // A part starts at 3 s, with a lookback of 2 s.
            let part = in_hz as usize * 3;
            let out = run_part(in_hz, &input, part, in_hz as usize * 2);
            assert_eq!(out.len(), whole.len() - 48_000, "{in_hz}");
            assert!(out == whole[48_000..], "{in_hz}: the samples differ");
        }
    }

    #[test]
    fn equal_rates_pass_through_untouched() {
        let input: Vec<f32> = (0..5_000).map(|i| i as f32 / 5_000.0).collect();
        assert_eq!(run(16_000, 16_000, &input, 333), input);
    }
}
