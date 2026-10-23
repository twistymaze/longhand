//! The mixer of Windows app audio (KTD4), kept free of WASAPI so that it is
//! tested on every platform.
//!
//! Each per-process loopback client sends packets at its own times, and a
//! client sends no packets while its app is silent. [`BlockMixer`] keeps
//! the samples of each client in a queue and sends their sum in blocks of
//! [`BLOCK`]. The wall clock sets the time of each block, so two clients
//! give one stream of real-time length. A client with no samples for a
//! block adds silence to it. After each mix, a queue keeps at most one
//! block of samples, so the stream does not become late.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// The rate of each client and of the mix.
pub const MIX_RATE: u32 = 48_000;

/// The length of one mixed block.
pub const BLOCK: Duration = Duration::from_millis(10);

/// The sum of the samples of each client, in blocks timed by the wall
/// clock.
pub struct BlockMixer {
    block_len: usize,
    queues: HashMap<u32, VecDeque<f32>>,
    next_end: Option<Instant>,
    block: Vec<f32>,
}

impl BlockMixer {
    /// A mixer of mono clients at `rate`.
    pub fn new(rate: u32) -> Self {
        let block_len = (rate as usize * BLOCK.as_millis() as usize / 1_000).max(1);
        Self {
            block_len,
            queues: HashMap::new(),
            next_end: None,
            block: vec![0.0; block_len],
        }
    }

    /// Add the samples that `client` sent.
    pub fn push(&mut self, client: u32, samples: &[f32]) {
        self.queues
            .entry(client)
            .or_default()
            .extend(samples.iter().copied());
    }

    /// Discard the samples of a client that stopped.
    pub fn remove(&mut self, client: u32) {
        self.queues.remove(&client);
    }

    /// Discard all samples and start the block clock again.
    pub fn clear(&mut self) {
        self.queues.clear();
        self.next_end = None;
    }

    /// Give each block that ended at or before `now` to `emit`. A block in
    /// which no client has samples is not sent.
    pub fn mix(&mut self, now: Instant, mut emit: impl FnMut(&[f32])) {
        let next_end = self.next_end.get_or_insert(now + BLOCK);
        while now >= *next_end {
            *next_end += BLOCK;
            self.block.fill(0.0);
            let mut any = false;
            for queue in self.queues.values_mut() {
                let n = queue.len().min(self.block_len);
                any |= n > 0;
                for (out, sample) in self.block.iter_mut().zip(queue.drain(..n)) {
                    *out += sample;
                }
            }
            if any {
                emit(&self.block);
            }
        }
        for queue in self.queues.values_mut() {
            let excess = queue.len().saturating_sub(self.block_len);
            queue.drain(..excess);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEN: usize = 480;

    fn ms(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    fn collect(mixer: &mut BlockMixer, now: Instant, out: &mut Vec<Vec<f32>>) {
        mixer.mix(now, |block| out.push(block.to_vec()));
    }

    #[test]
    fn two_clients_with_frames_at_the_same_time_give_their_sum() {
        let start = Instant::now();
        let mut mixer = BlockMixer::new(MIX_RATE);
        let mut out = Vec::new();
        collect(&mut mixer, start, &mut out);
        mixer.push(1, &[0.1; LEN]);
        mixer.push(2, &[0.25; LEN]);
        collect(&mut mixer, ms(start, 10), &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].len(), LEN);
        assert!(out[0].iter().all(|s| (s - 0.35).abs() < 1e-6), "{out:?}");
    }

    #[test]
    fn a_client_with_no_packets_adds_nothing() {
        let start = Instant::now();
        let mut mixer = BlockMixer::new(MIX_RATE);
        let mut out = Vec::new();
        collect(&mut mixer, start, &mut out);
        mixer.push(1, &[0.2; LEN]);
        mixer.push(2, &[]);
        collect(&mut mixer, ms(start, 10), &mut out);
        assert_eq!(out, vec![vec![0.2; LEN]]);
    }

    #[test]
    fn a_block_with_no_samples_is_not_sent() {
        let start = Instant::now();
        let mut mixer = BlockMixer::new(MIX_RATE);
        let mut out = Vec::new();
        collect(&mut mixer, start, &mut out);
        mixer.push(1, &[]);
        collect(&mut mixer, ms(start, 50), &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn a_short_packet_is_filled_with_silence_to_one_block() {
        let start = Instant::now();
        let mut mixer = BlockMixer::new(MIX_RATE);
        let mut out = Vec::new();
        collect(&mut mixer, start, &mut out);
        mixer.push(1, &[0.5; 100]);
        collect(&mut mixer, ms(start, 10), &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0][..100], [0.5; 100]);
        assert!(out[0][100..].iter().all(|s| *s == 0.0));
    }

    #[test]
    fn two_clients_5_ms_apart_give_one_stream_of_real_time_length() {
        let start = Instant::now();
        let mut mixer = BlockMixer::new(MIX_RATE);
        let mut out = Vec::new();
        collect(&mut mixer, start, &mut out);
        // One second: client 1 sends a 10 ms packet at 0, 10, 20 ms and
        // so on, and client 2 sends one at 5, 15, 25 ms and so on. The
        // mixer runs after each packet.
        for t in (0..1_000).step_by(5) {
            if t % 10 == 0 {
                mixer.push(1, &[0.1; LEN]);
            } else {
                mixer.push(2, &[0.2; LEN]);
            }
            collect(&mut mixer, ms(start, t), &mut out);
        }
        collect(&mut mixer, ms(start, 1_000), &mut out);
        let samples: usize = out.iter().map(Vec::len).sum();
        assert_eq!(samples, MIX_RATE as usize, "one second, not two");
        let summed = out
            .iter()
            .flatten()
            .filter(|s| (*s - 0.3).abs() < 1e-6)
            .count();
        assert!(
            summed >= samples - 2 * LEN,
            "the two clients play at the same time: {summed} of {samples} samples are the sum"
        );
    }

    #[test]
    fn samples_older_than_one_block_are_dropped() {
        let start = Instant::now();
        let mut mixer = BlockMixer::new(MIX_RATE);
        let mut out = Vec::new();
        collect(&mut mixer, start, &mut out);
        // A burst of four blocks arrives in the first block.
        let burst: Vec<f32> = (0..4 * LEN).map(|i| (i / LEN) as f32).collect();
        mixer.push(1, &burst);
        collect(&mut mixer, ms(start, 10), &mut out);
        collect(&mut mixer, ms(start, 20), &mut out);
        collect(&mut mixer, ms(start, 30), &mut out);
        assert_eq!(out.len(), 2, "the oldest block and the newest block only");
        assert!(out[0].iter().all(|s| *s == 0.0));
        assert!(out[1].iter().all(|s| *s == 3.0));
    }

    #[test]
    fn a_removed_client_adds_nothing() {
        let start = Instant::now();
        let mut mixer = BlockMixer::new(MIX_RATE);
        let mut out = Vec::new();
        collect(&mut mixer, start, &mut out);
        mixer.push(1, &[0.1; LEN]);
        mixer.push(2, &[0.2; LEN]);
        mixer.remove(2);
        collect(&mut mixer, ms(start, 10), &mut out);
        assert_eq!(out, vec![vec![0.1; LEN]]);
    }
}
