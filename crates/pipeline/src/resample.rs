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
        }
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
        let data = &data[..data.len().min(room)];
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

    #[test]
    fn equal_rates_pass_through_untouched() {
        let input: Vec<f32> = (0..5_000).map(|i| i as f32 / 5_000.0).collect();
        assert_eq!(run(16_000, 16_000, &input, 333), input);
    }
}
