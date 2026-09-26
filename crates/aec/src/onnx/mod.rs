use realfft::{ComplexToReal, RealFftPlanner, RealToComplex, num_complex::Complex};
use std::sync::Arc;

use ::onnx::{
    ndarray::{Array3, Array4, ErrorKind, ShapeError},
    ort::{session::Session, value::TensorRef},
};

use super::CircularBuffer;

pub(crate) mod model;

// On the development machine, 30 minutes of audio take 81 s with one thread
// and 37 s with four. More than four threads give almost no gain.
const MAX_INTRA_THREADS: usize = 4;

fn load_session(bytes: &[u8]) -> Result<Session, crate::Error> {
    let threads = std::thread::available_parallelism()
        .map_or(1, |count| count.get())
        .min(MAX_INTRA_THREADS);
    Ok(::onnx::load_model_from_bytes_with_threads(bytes, threads)?)
}

fn layout_error() -> crate::Error {
    crate::Error::Shape(ShapeError::from_kind(ErrorKind::IncompatibleLayout))
}

struct ProcessingContext {
    scratch: Vec<Complex<f32>>,
    ifft_scratch: Vec<Complex<f32>>,
    in_buffer_fft: Vec<f32>,
    in_block_fft: Vec<Complex<f32>>,
    lpb_buffer_fft: Vec<f32>,
    lpb_block_fft: Vec<Complex<f32>>,
    estimated_block_vec: Vec<f32>,
    in_mag: Array3<f32>,
    lpb_mag: Array3<f32>,
    estimated_block: Array3<f32>,
    in_lpb: Array3<f32>,
    out_mask: Vec<f32>,
    out_block: Vec<f32>,
}

impl ProcessingContext {
    fn new(
        block_len: usize,
        fft: &Arc<dyn RealToComplex<f32>>,
        ifft: &Arc<dyn ComplexToReal<f32>>,
    ) -> Self {
        Self {
            scratch: vec![Complex::new(0.0f32, 0.0f32); fft.get_scratch_len()],
            ifft_scratch: vec![Complex::new(0.0f32, 0.0f32); ifft.get_scratch_len()],
            in_buffer_fft: vec![0.0f32; block_len],
            in_block_fft: vec![Complex::new(0.0f32, 0.0f32); block_len / 2 + 1],
            lpb_buffer_fft: vec![0.0f32; block_len],
            lpb_block_fft: vec![Complex::new(0.0f32, 0.0f32); block_len / 2 + 1],
            estimated_block_vec: vec![0.0f32; block_len],
            in_mag: Array3::<f32>::zeros((1, 1, block_len / 2 + 1)),
            lpb_mag: Array3::<f32>::zeros((1, 1, block_len / 2 + 1)),
            estimated_block: Array3::<f32>::zeros((1, 1, block_len)),
            in_lpb: Array3::<f32>::zeros((1, 1, block_len)),
            out_mask: vec![0.0f32; block_len / 2 + 1],
            out_block: vec![0.0f32; block_len],
        }
    }
}

/// The two-stage DTLN-aec model. `lpb` is the loopback signal: the reference
/// that the speakers play.
pub(crate) struct Dtln {
    session_1: Session,
    session_2: Session,
    block_len: usize,
    block_shift: usize,
    fft: Arc<dyn RealToComplex<f32>>,
    ifft: Arc<dyn ComplexToReal<f32>>,
    // The state between streaming calls.
    states_1: Array4<f32>,
    states_2: Array4<f32>,
    in_buffer: CircularBuffer,
    in_buffer_lpb: CircularBuffer,
    out_buffer: CircularBuffer,
}

impl Dtln {
    pub(crate) fn new() -> Result<Self, crate::Error> {
        let (block_len, block_shift) = (model::BLOCK_SIZE, model::BLOCK_SHIFT);

        let mut fft_planner = RealFftPlanner::<f32>::new();
        let fft = fft_planner.plan_fft_forward(block_len);
        let ifft = fft_planner.plan_fft_inverse(block_len);

        let session_1 = load_session(model::BYTES_1)?;
        let session_2 = load_session(model::BYTES_2)?;

        let state_size = model::STATE_SIZE;

        Ok(Dtln {
            session_1,
            session_2,
            block_len,
            block_shift,
            fft,
            ifft,
            states_1: Array4::<f32>::zeros((1, 2, state_size, 2)),
            states_2: Array4::<f32>::zeros((1, 2, state_size, 2)),
            in_buffer: CircularBuffer::new(block_len, block_shift),
            in_buffer_lpb: CircularBuffer::new(block_len, block_shift),
            out_buffer: CircularBuffer::new(block_len, block_shift),
        })
    }

    pub(crate) fn reset(&mut self) {
        let state_size = model::STATE_SIZE;
        self.states_1 = Array4::<f32>::zeros((1, 2, state_size, 2));
        self.states_2 = Array4::<f32>::zeros((1, 2, state_size, 2));
        self.in_buffer.clear();
        self.in_buffer_lpb.clear();
        self.out_buffer.clear();
    }

    fn calculate_fft_magnitude(
        &self,
        input: &[f32],
        fft_buffer: &mut [f32],
        fft_result: &mut [Complex<f32>],
        scratch: &mut [Complex<f32>],
        magnitude: &mut Array3<f32>,
    ) -> Result<(), crate::Error> {
        fft_buffer.copy_from_slice(input);
        self.fft
            .process_with_scratch(fft_buffer, fft_result, scratch)?;

        for (i, &c) in fft_result.iter().enumerate() {
            magnitude[[0, 0, i]] = c.norm();
        }

        Ok(())
    }

    fn run_model_1(&mut self, ctx: &mut ProcessingContext) -> Result<(), crate::Error> {
        let mut outputs = self.session_1.run(::onnx::ort::inputs![
            TensorRef::from_array_view(ctx.in_mag.view())?,
            TensorRef::from_array_view(self.states_1.view())?,
            TensorRef::from_array_view(ctx.lpb_mag.view())?
        ])?;

        let out_mask = outputs
            .remove("Identity")
            .ok_or_else(|| crate::Error::MissingOutput("Identity".to_string()))?;
        let out_mask_view = out_mask.try_extract_array::<f32>()?;
        ctx.out_mask
            .copy_from_slice(out_mask_view.view().as_slice().ok_or_else(layout_error)?);

        let new_states = outputs
            .remove("Identity_1")
            .ok_or_else(|| crate::Error::MissingOutput("Identity_1".to_string()))?;
        let new_states_view = new_states.try_extract_array::<f32>()?;
        self.states_1
            .as_slice_mut()
            .ok_or_else(layout_error)?
            .copy_from_slice(new_states_view.view().as_slice().ok_or_else(layout_error)?);

        Ok(())
    }

    fn run_model_2(&mut self, ctx: &mut ProcessingContext) -> Result<(), crate::Error> {
        let mut outputs = self.session_2.run(::onnx::ort::inputs![
            TensorRef::from_array_view(ctx.estimated_block.view())?,
            TensorRef::from_array_view(self.states_2.view())?,
            TensorRef::from_array_view(ctx.in_lpb.view())?
        ])?;

        let out_block = outputs
            .remove("Identity")
            .ok_or_else(|| crate::Error::MissingOutput("Identity".into()))?;
        let out_block_view = out_block.try_extract_array::<f32>()?;
        ctx.out_block
            .copy_from_slice(out_block_view.view().as_slice().ok_or_else(layout_error)?);

        let new_states = outputs
            .remove("Identity_1")
            .ok_or_else(|| crate::Error::MissingOutput("Identity_1".into()))?;
        let new_states_view = new_states.try_extract_array::<f32>()?;
        self.states_2
            .as_slice_mut()
            .ok_or_else(layout_error)?
            .copy_from_slice(new_states_view.view().as_slice().ok_or_else(layout_error)?);

        Ok(())
    }

    /// Processes one chunk and keeps the model state for the next chunk. The
    /// output is late by `BLOCK_SIZE - BLOCK_SHIFT` samples. The caller makes
    /// sure that the chunk length is a multiple of `BLOCK_SHIFT`.
    pub(crate) fn process_streaming(
        &mut self,
        mic_input: &[f32],
        lpb_input: &[f32],
    ) -> Result<Vec<f32>, crate::Error> {
        let len_audio = mic_input.len().min(lpb_input.len());
        if len_audio == 0 {
            return Ok(vec![]);
        }

        self.process_blocks(&mic_input[..len_audio], &lpb_input[..len_audio])
    }

    fn process_blocks(&mut self, audio: &[f32], lpb: &[f32]) -> Result<Vec<f32>, crate::Error> {
        let mut out_file = vec![0.0f32; audio.len()];
        let num_blocks = audio.len() / self.block_shift;

        let mut ctx = ProcessingContext::new(self.block_len, &self.fft, &self.ifft);

        for idx in 0..num_blocks {
            // Shift the buffers and write the next part of the input.
            let start = idx * self.block_shift;
            let end = (start + self.block_shift).min(audio.len());
            self.in_buffer.push_chunk(&audio[start..end]);
            self.in_buffer_lpb.push_chunk(&lpb[start..end]);

            self.calculate_fft_magnitude(
                self.in_buffer.data(),
                &mut ctx.in_buffer_fft,
                &mut ctx.in_block_fft,
                &mut ctx.scratch,
                &mut ctx.in_mag,
            )?;
            self.calculate_fft_magnitude(
                self.in_buffer_lpb.data(),
                &mut ctx.lpb_buffer_fft,
                &mut ctx.lpb_block_fft,
                &mut ctx.scratch,
                &mut ctx.lpb_mag,
            )?;

            self.run_model_1(&mut ctx)?;

            // Apply the mask, then go back to the time domain.
            for (c, &m) in ctx.in_block_fft.iter_mut().zip(ctx.out_mask.iter()) {
                *c *= m;
            }
            self.ifft.process_with_scratch(
                &mut ctx.in_block_fft,
                &mut ctx.estimated_block_vec,
                &mut ctx.ifft_scratch,
            )?;

            // Normalize the inverse FFT and copy it to the input of stage 2.
            let norm_factor = 1.0 / self.block_len as f32;
            let est_slice = ctx
                .estimated_block
                .as_slice_mut()
                .ok_or_else(layout_error)?;
            for (d, &s) in est_slice.iter_mut().zip(ctx.estimated_block_vec.iter()) {
                *d = s * norm_factor;
            }
            ctx.in_lpb
                .as_slice_mut()
                .ok_or_else(layout_error)?
                .copy_from_slice(self.in_buffer_lpb.data());

            self.run_model_2(&mut ctx)?;

            self.out_buffer.shift_and_accumulate(&ctx.out_block);

            let out_start = idx * self.block_shift;
            let out_end = (out_start + self.block_shift).min(out_file.len());
            let out_chunk_len = out_end - out_start;
            out_file[out_start..out_end].copy_from_slice(&self.out_buffer.data()[..out_chunk_len]);
        }

        Ok(out_file)
    }
}
