use super::{VadFrame, VadTailReport, VoiceActivityDetector};
use anyhow::Result;
use std::collections::VecDeque;

/// One pre-roll buffer slot. `emitted` keeps the pre-roll from sending a
/// frame again, and `tail_report()` uses it. Only `tail_report()` uses
/// `voiced`.
struct BufferedFrame {
    samples: Vec<f32>,
    emitted: bool,
    voiced: bool,
}

pub struct SmoothedVad {
    inner_vad: Box<dyn VoiceActivityDetector>,
    prefill_frames: usize,
    hangover_frames: usize,
    onset_frames: usize,

    frame_buffer: VecDeque<BufferedFrame>,
    hangover_counter: usize,
    onset_counter: usize,
    in_speech: bool,

    temp_out: Vec<f32>,
}

impl SmoothedVad {
    pub fn new(
        inner_vad: Box<dyn VoiceActivityDetector>,
        prefill_frames: usize,
        hangover_frames: usize,
        onset_frames: usize,
    ) -> Self {
        Self {
            inner_vad,
            prefill_frames,
            hangover_frames,
            onset_frames,
            frame_buffer: VecDeque::new(),
            hangover_counter: 0,
            onset_counter: 0,
            in_speech: false,
            temp_out: Vec::new(),
        }
    }

    fn mark_last_emitted(&mut self) {
        if let Some(frame) = self.frame_buffer.back_mut() {
            frame.emitted = true;
        }
    }
}

impl VoiceActivityDetector for SmoothedVad {
    fn push_frame<'a>(&'a mut self, frame: &'a [f32]) -> Result<VadFrame<'a>> {
        // 1. Buffer every incoming frame for possible pre-roll
        self.frame_buffer.push_back(BufferedFrame {
            samples: frame.to_vec(),
            emitted: false,
            voiced: false,
        });
        while self.frame_buffer.len() > self.prefill_frames + 1 {
            self.frame_buffer.pop_front();
        }

        // 2. Delegate to the wrapped boolean VAD
        let is_voice = self.inner_vad.is_voice(frame)?;
        if let Some(last) = self.frame_buffer.back_mut() {
            last.voiced = is_voice;
        }

        match (self.in_speech, is_voice) {
            // Potential start of speech - need to accumulate onset frames
            (false, true) => {
                self.onset_counter += 1;
                if self.onset_counter >= self.onset_frames {
                    // We have enough consecutive voice frames to trigger speech
                    self.in_speech = true;
                    self.hangover_counter = self.hangover_frames;
                    self.onset_counter = 0; // Reset for next time

                    // Collect prefill + current frame
                    self.temp_out.clear();
                    for buffered in self.frame_buffer.iter_mut().filter(|f| !f.emitted) {
                        self.temp_out.extend(buffered.samples.iter());
                        buffered.emitted = true;
                    }
                    Ok(VadFrame::Speech(&self.temp_out))
                } else {
                    // Not enough frames yet, still silence
                    Ok(VadFrame::Noise)
                }
            }

            // Ongoing Speech
            (true, true) => {
                self.hangover_counter = self.hangover_frames;
                self.mark_last_emitted();
                Ok(VadFrame::Speech(frame))
            }

            // End of Speech or interruption during onset phase
            (true, false) => {
                if self.hangover_counter > 0 {
                    self.hangover_counter -= 1;
                    self.mark_last_emitted();
                    Ok(VadFrame::Speech(frame))
                } else {
                    self.in_speech = false;
                    Ok(VadFrame::Noise)
                }
            }

            // Silence or broken onset sequence
            (false, false) => {
                self.onset_counter = 0; // Reset onset counter on silence
                Ok(VadFrame::Noise)
            }
        }
    }

    fn frame_samples(&self) -> usize {
        self.inner_vad.frame_samples()
    }

    fn set_hangover_frames(&mut self, frames: usize) {
        self.hangover_frames = frames;
    }

    /// Trailing run of withheld frames plus smoothing state. Interior
    /// withheld frames (before already-emitted speech) are not counted.
    fn tail_report(&self) -> Option<VadTailReport> {
        let mut withheld_frames = 0;
        let mut withheld_voiced_frames = 0;
        for frame in self
            .frame_buffer
            .iter()
            .rev()
            .take_while(|frame| !frame.emitted)
        {
            withheld_frames += 1;
            if frame.voiced {
                withheld_voiced_frames += 1;
            }
        }

        Some(VadTailReport {
            withheld_frames,
            withheld_voiced_frames,
            in_speech: self.in_speech,
            onset_counter: self.onset_counter,
            hangover_counter: self.hangover_counter,
        })
    }

    fn reset(&mut self) {
        self.inner_vad.reset();
        self.frame_buffer.clear();
        self.hangover_counter = 0;
        self.onset_counter = 0;
        self.in_speech = false;
        self.temp_out.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Inner VAD that replays a scripted voice/no-voice sequence.
    struct ScriptedVad {
        script: VecDeque<bool>,
    }

    impl ScriptedVad {
        fn new(script: &[bool]) -> Self {
            Self {
                script: script.iter().copied().collect(),
            }
        }
    }

    impl VoiceActivityDetector for ScriptedVad {
        fn push_frame<'a>(&'a mut self, frame: &'a [f32]) -> Result<VadFrame<'a>> {
            if self.script.pop_front().unwrap_or(false) {
                Ok(VadFrame::Speech(frame))
            } else {
                Ok(VadFrame::Noise)
            }
        }

        fn frame_samples(&self) -> usize {
            4
        }
    }

    fn frame(value: f32) -> Vec<f32> {
        vec![value; 4]
    }

    fn smoothed(script: &[bool], onset_frames: usize) -> SmoothedVad {
        SmoothedVad::new(Box::new(ScriptedVad::new(script)), 3, 2, onset_frames)
    }

    #[test]
    fn tail_report_counts_withheld_onset_tail() {
        // One voiced frame at the end: onset (2 frames) never confirms, so
        // both trailing frames are withheld and one of them is voiced —
        // consistent with a final word cut off at the stop boundary.
        let mut vad = smoothed(&[false, true], 2);
        assert!(!vad.push_frame(&frame(0.1)).unwrap().is_speech());
        assert!(!vad.push_frame(&frame(0.9)).unwrap().is_speech());

        let report = vad.tail_report().expect("smoothed VAD always reports");
        assert_eq!(report.withheld_frames, 2);
        assert_eq!(report.withheld_voiced_frames, 1);
        assert_eq!(report.onset_counter, 1);
        assert!(!report.in_speech);
    }

    #[test]
    fn tail_report_counts_only_trailing_run() {
        // Speech emitted through hangover, then silence past the hangover is
        // withheld. Only the trailing run counts — frames older than an
        // emitted frame are excluded.
        let mut vad = smoothed(&[true, true, false, false, false, false], 2);
        for v in [0.1, 0.2, 0.3, 0.4] {
            let _ = vad.push_frame(&frame(v)).unwrap();
        }
        assert!(!vad.push_frame(&frame(0.5)).unwrap().is_speech()); // past hangover
        assert!(!vad.push_frame(&frame(0.6)).unwrap().is_speech());

        let report = vad.tail_report().unwrap();
        assert_eq!(report.withheld_frames, 2);
        assert_eq!(report.withheld_voiced_frames, 0);
    }

    /// The first sample of each frame in `out`. Each test frame has one value.
    fn frame_values(out: &[f32]) -> Vec<f32> {
        out.chunks(4).map(|chunk| chunk[0]).collect()
    }

    #[test]
    fn first_onset_sends_the_full_pre_roll() {
        let mut vad = smoothed(&[false, false, true, true], 2);
        for v in [1.0, 2.0, 3.0] {
            assert!(!vad.push_frame(&frame(v)).unwrap().is_speech());
        }
        let last = frame(4.0);
        let VadFrame::Speech(out) = vad.push_frame(&last).unwrap() else {
            panic!("onset must send speech");
        };
        assert_eq!(frame_values(out), [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn second_onset_does_not_send_hangover_frames_again() {
        // Speech (1-2), hangover (3-4), one noise frame (5), then speech
        // again (6-7) inside the pre-roll window. The pre-roll at the second
        // onset holds frame 4, which the hangover already sent.
        let mut vad = smoothed(&[true, true, false, false, false, true, true], 2);
        let mut sent = Vec::new();
        for v in 1..=7 {
            if let VadFrame::Speech(out) = vad.push_frame(&frame(v as f32)).unwrap() {
                sent.extend(frame_values(out));
            }
        }
        assert_eq!(sent, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]);
    }
}
