use anlg_onnx::{
    ndarray::{self, Axis},
    ort::{self, session::Session, value::TensorRef},
};

pub const SAMPLE_RATE: u32 = 16_000;
/// The model is trained on 10 s windows and only accepts that length.
pub const WINDOW_SAMPLES: usize = SAMPLE_RATE as usize * 10;
/// Each output frame summarises this many input samples.
pub const FRAME_SIZE: usize = 270;
/// Receptive-field offset of the first output frame.
pub const FRAME_START: usize = 721;
/// The model separates at most this many concurrent speakers inside a window.
pub const LOCAL_SPEAKERS: usize = 3;

// segmentation-3.0 predicts a powerset over three local speakers, ordered by
// cardinality: silence, the three singletons, then the three pairs.
const POWERSET: [[bool; LOCAL_SPEAKERS]; 7] = [
    [false, false, false],
    [true, false, false],
    [false, true, false],
    [false, false, true],
    [true, true, false],
    [true, false, true],
    [false, true, true],
];

/// Per-frame activity of the local speakers inside one 10 s window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowActivity {
    pub frames: Vec<[bool; LOCAL_SPEAKERS]>,
}

impl WindowActivity {
    pub fn active_frames(&self, speaker: usize) -> usize {
        self.frames.iter().filter(|frame| frame[speaker]).count()
    }

    /// Frames where `speaker` talks alone; overlapped frames contaminate
    /// speaker embeddings.
    pub fn clean_frames(&self, speaker: usize) -> usize {
        self.frames
            .iter()
            .filter(|frame| frame[speaker] && frame.iter().filter(|active| **active).count() == 1)
            .count()
    }

    pub fn speech_frames(&self) -> usize {
        self.frames
            .iter()
            .filter(|frame| frame.iter().any(|active| *active))
            .count()
    }
}

pub fn frame_start_sample(frame: usize) -> usize {
    FRAME_START + frame * FRAME_SIZE
}

/// Number of output frames fully covered by `samples` valid input samples.
pub fn frames_for_samples(samples: usize) -> usize {
    samples.saturating_sub(FRAME_START) / FRAME_SIZE
}

pub struct Segmenter {
    session: Session,
}

impl Segmenter {
    /// Load the weights from `path`, the models directory's copy (R28).
    pub fn from_path(path: impl AsRef<std::path::Path>) -> Result<Self, crate::Error> {
        let session = anlg_onnx::load_model_from_path(path)?;
        Ok(Self { session })
    }

    /// Runs one window. `window` must hold exactly [`WINDOW_SAMPLES`] samples in
    /// `[-1, 1]`; the model normalises the waveform so scale does not matter.
    pub fn run_window(&mut self, window: &[f32]) -> Result<WindowActivity, crate::Error> {
        if window.len() != WINDOW_SAMPLES {
            return Err(crate::Error::WindowLength {
                expected: WINDOW_SAMPLES,
                actual: window.len(),
            });
        }

        let array = ndarray::Array1::from_iter(window.iter().copied())
            .insert_axis(Axis(0))
            .insert_axis(Axis(1))
            .into_dyn();
        let inputs = ort::inputs![TensorRef::from_array_view(array.view())?];
        let run_output = self.session.run(inputs)?;
        let output_tensor = run_output
            .values()
            .next()
            .ok_or(crate::Error::EmptyRowError)?;
        let outputs = output_tensor.try_extract_array::<f32>()?;

        let mut frames = Vec::new();
        for batch in outputs.outer_iter() {
            for row in batch.axis_iter(Axis(0)) {
                let class = row
                    .iter()
                    .enumerate()
                    .max_by(|(_, a), (_, b)| a.total_cmp(b))
                    .map(|(index, _)| index)
                    .ok_or(crate::Error::EmptyRowError)?;
                frames.push(POWERSET[class.min(POWERSET.len() - 1)]);
            }
        }

        Ok(WindowActivity { frames })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `LONGHAND_WEIGHTS_DIR` must hold `segmentation.onnx`.
    fn segmenter() -> Segmenter {
        let dir = std::env::var("LONGHAND_WEIGHTS_DIR").expect("LONGHAND_WEIGHTS_DIR is set");
        Segmenter::from_path(std::path::Path::new(&dir).join("segmentation.onnx")).unwrap()
    }

    #[test]
    #[ignore = "needs the segmentation weights on disk"]
    fn silence_produces_no_speech() {
        let window = vec![0.0f32; WINDOW_SAMPLES];
        let activity = segmenter().run_window(&window).unwrap();
        assert_eq!(activity.frames.len(), frames_for_samples(WINDOW_SAMPLES));
        assert_eq!(activity.speech_frames(), 0);
    }

    #[test]
    #[ignore = "needs the segmentation weights on disk"]
    fn rejects_wrong_window_length() {
        assert!(segmenter().run_window(&[0.0; 10]).is_err());
    }

    #[test]
    fn a_missing_weights_file_is_an_error_not_a_panic() {
        assert!(Segmenter::from_path("/nonexistent/segmentation.onnx").is_err());
    }
}
