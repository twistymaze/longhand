//! The echo canceller: it removes a reference signal (the system audio that
//! the speakers play) from a microphone signal. It runs the DTLN-aec model on
//! the CPU and then the linear residual step.

mod error;
mod linear;
mod onnx;

pub use error::Error;
pub use linear::cancel_linear_echo;

/// The sample rate of the model, in Hz. Both inputs must use it.
pub const SAMPLE_RATE: u32 = 16_000;
/// The length of one model block, in samples.
pub const BLOCK_SIZE: usize = onnx::model::BLOCK_SIZE;
/// The number of new samples in each model block. A chunk length must be a
/// multiple of this value.
pub const BLOCK_SHIFT: usize = onnx::model::BLOCK_SHIFT;
/// The output of `EchoCanceller::cancel_chunk` is late by this number of
/// samples, compared with its input.
pub const STREAMING_LATENCY_SAMPLES: usize = BLOCK_SIZE - BLOCK_SHIFT;

/// A streaming echo canceller. It keeps the model state and the linear gain
/// between chunks, so the caller gives it the chunks of one signal in order.
pub struct EchoCanceller {
    model: onnx::Dtln,
    /// The last `STREAMING_LATENCY_SAMPLES` reference samples. The linear
    /// step uses them to align the reference with the late model output.
    reference_history: Vec<f32>,
    linear_echo_gain: Option<f32>,
}

impl EchoCanceller {
    /// Loads the two embedded models.
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            model: onnx::Dtln::new()?,
            reference_history: vec![0.0; STREAMING_LATENCY_SAMPLES],
            linear_echo_gain: None,
        })
    }

    /// Clears the model state and the linear gain. Call it before a signal
    /// that does not continue the previous chunks, or when the alignment of
    /// the reference changes.
    pub fn reset(&mut self) {
        self.model.reset();
        self.reference_history.fill(0.0);
        self.linear_echo_gain = None;
    }

    /// Removes the reference from one chunk of the microphone signal.
    ///
    /// Both chunks are 16 kHz mono, have the same length, and the length is a
    /// multiple of `BLOCK_SHIFT`. The reference must be aligned with the echo
    /// in the microphone signal to within a few milliseconds. The output has
    /// the chunk length and is late by `STREAMING_LATENCY_SAMPLES`.
    pub fn cancel_chunk(&mut self, mic: &[f32], reference: &[f32]) -> Result<Vec<f32>, Error> {
        if mic.len() != reference.len() || !mic.len().is_multiple_of(BLOCK_SHIFT) {
            return Err(Error::ChunkLength {
                mic: mic.len(),
                reference: reference.len(),
            });
        }
        if mic.is_empty() {
            return Ok(Vec::new());
        }

        let processed = self.model.process_streaming(mic, reference)?;

        // The model output is late, so the linear step uses the reference
        // that is late by the same number of samples.
        let mut late_reference = Vec::with_capacity(reference.len() + STREAMING_LATENCY_SAMPLES);
        late_reference.extend_from_slice(&self.reference_history);
        late_reference.extend_from_slice(reference);
        let history_start = late_reference.len() - STREAMING_LATENCY_SAMPLES;
        self.reference_history
            .copy_from_slice(&late_reference[history_start..]);
        late_reference.truncate(reference.len());

        Ok(cancel_linear_echo(
            &late_reference,
            processed,
            &mut self.linear_echo_gain,
        ))
    }
}

pub(crate) struct CircularBuffer {
    buffer: Vec<f32>,
    block_len: usize,
    block_shift: usize,
}

impl CircularBuffer {
    fn new(block_len: usize, block_shift: usize) -> Self {
        Self {
            buffer: vec![0.0f32; block_len],
            block_len,
            block_shift,
        }
    }

    fn push_chunk(&mut self, chunk: &[f32]) {
        let keep = self.block_len - self.block_shift;
        self.buffer.copy_within(self.block_shift.., 0);
        let copy_len = chunk.len().min(self.block_shift);
        self.buffer[keep..keep + copy_len].copy_from_slice(&chunk[..copy_len]);

        if copy_len < self.block_shift {
            self.buffer[keep + copy_len..].fill(0.0);
        }
    }

    fn shift_and_accumulate(&mut self, data: &[f32]) {
        let keep = self.block_len - self.block_shift;
        self.buffer.copy_within(self.block_shift.., 0);
        self.buffer[keep..].fill(0.0);

        for (d, &val) in self.buffer.iter_mut().zip(data.iter()) {
            *d += val;
        }
    }

    fn data(&self) -> &[f32] {
        &self.buffer
    }

    fn clear(&mut self) {
        self.buffer.fill(0.0);
    }
}
