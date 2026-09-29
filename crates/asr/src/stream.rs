//! Live recognition in parts (KTD4, KTD5).
//!
//! A `transcribe-cpp` stream keeps all of its audio and decodes all of its
//! tokens again on each change, so one stream for a whole meeting grows
//! without limit. [`Parts`] ends a stream at the first pause of at least
//! 600 ms after 4 minutes, and always at 8 minutes, and the caller starts the
//! next part on a new stream. After a cut at 8 minutes, the last 2 s of audio
//! go into the next part again, and the two parts divide that audio by word
//! time, so each word is kept once.
//!
//! A stream gives timed tokens only. [`words_from_tokens`] builds the words.
//!
//! A stream borrows its recognizer, so the caller holds the recognizer, the
//! stream and the [`Parts`]. [`stream_all`] shows the loop.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use vad::{EarshotVad, VoiceActivityDetector};

use crate::windows::{FRAME_MS, FRAME_SAMPLES, THRESHOLD};
use crate::{AsrError, Recognizer, SAMPLES_PER_MS};

/// One recognized token, in milliseconds from the start of its stream or
/// window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimedToken {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// One word, or one character group of a script without spaces, in
/// milliseconds from the start of the segment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveWord {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// One finished part of a live stream: the span of segment audio that the
/// stream heard, and its words. After a cut at the limit, a part starts in
/// the replay, so its span overlaps the span of the part before it; the words
/// do not overlap.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LivePart {
    pub start_ms: u64,
    pub end_ms: u64,
    pub words: Vec<LiveWord>,
}

/// The text of the open part: `committed` does not change again, and
/// `tentative` can change on the next feed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LiveText {
    pub committed: String,
    pub tentative: String,
}

/// A live recognition stream over mono 16 kHz audio.
pub trait LiveStream: Send {
    /// Feed audio. Returns true when the text changed.
    fn feed(&mut self, pcm: &[f32]) -> Result<bool, AsrError>;
    fn text(&self) -> LiveText;
    /// End the input and return every token, in milliseconds from the
    /// start of the stream.
    fn finish(&mut self) -> Result<Vec<TimedToken>, AsrError>;
    /// Discard the stream's state without a result.
    fn reset(&mut self);
    /// The timed words of the open part so far, in milliseconds from the
    /// start of the stream. The stream continues to take audio. The words
    /// are built with [`words_from_tokens`], as [`Parts::finish`] builds the
    /// words of a finished part. A stream that has no word times gives no
    /// words; this is the default.
    fn open_words(&self) -> Vec<LiveWord> {
        Vec::new()
    }
}

/// Characters of scripts that put no spaces between words: CJK
/// punctuation, kana, CJK ideographs and full-width forms.
pub fn unspaced(c: char) -> bool {
    matches!(
        c,
        '\u{3000}'..='\u{30FF}'
            | '\u{31F0}'..='\u{31FF}'
            | '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{FF00}'..='\u{FFEF}'
            | '\u{20000}'..='\u{2FFFF}'
    )
}

/// Group tokens into words and add `offset_ms` to their times. A token that
/// starts with a space (or the SentencePiece marker `▁`) starts a new word;
/// other tokens continue the word before them. In a script without spaces,
/// each token is its own timed unit, so the speaker split can still cut at a
/// turn change. A word has the start of its first token and the end of its
/// last token.
pub fn words_from_tokens(tokens: &[TimedToken], offset_ms: u64) -> Vec<LiveWord> {
    let mut words: Vec<LiveWord> = Vec::new();
    // Whether the next token can continue the last word.
    let mut open = false;
    for token in tokens {
        let raw = token.text.replace('\u{2581}', " ");
        if raw.is_empty() {
            continue;
        }
        let text = raw.trim();
        if text.is_empty() {
            open = false;
            continue;
        }
        let start_ms = offset_ms + token.start_ms;
        let end_ms = offset_ms + token.end_ms.max(token.start_ms);
        let continues = open
            && !raw.starts_with(char::is_whitespace)
            && !text.starts_with(unspaced)
            && words.last().is_some_and(|w| !w.text.ends_with(unspaced));
        match words.last_mut() {
            Some(word) if continues => {
                word.text.push_str(text);
                word.end_ms = word.end_ms.max(end_ms);
            }
            _ => words.push(LiveWord {
                start_ms,
                end_ms,
                text: text.to_string(),
            }),
        }
        open = !raw.ends_with(char::is_whitespace);
    }
    words
}

/// The text of `words`, with a space between two words unless one side is a
/// script without spaces.
pub fn join_words(words: &[LiveWord]) -> String {
    let mut text = String::new();
    for word in words {
        if !text.is_empty() && !text.ends_with(unspaced) && !word.text.starts_with(unspaced) {
            text.push(' ');
        }
        text.push_str(&word.text);
    }
    text
}

/// When a part ends (KTD4). The implementer may change the durations after
/// a measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartPolicy {
    /// A part does not end at a pause before this length.
    pub min_ms: u64,
    /// A part always ends at this length, the replay included.
    pub max_ms: u64,
    /// The silence that counts as a pause.
    pub pause_ms: u64,
    /// The audio that goes into the next part again after a cut at the
    /// limit.
    pub replay_ms: u64,
}

impl Default for PartPolicy {
    fn default() -> Self {
        Self {
            min_ms: 4 * 60_000,
            max_ms: 8 * 60_000,
            pause_ms: 600,
            replay_ms: 2_000,
        }
    }
}

/// Why a part ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cut {
    /// A pause after the minimum length. No audio is replayed.
    Pause,
    /// The maximum length. The next part replays the last audio.
    Limit,
}

/// The result of one [`Parts::feed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Fed {
    /// The stream's text changed: read it with [`LiveStream::text`].
    pub changed: bool,
    /// The part must end now: call [`Parts::finish`], then feed the next
    /// audio into a new stream.
    pub cut: Option<Cut>,
}

type Classifier = Box<dyn FnMut(&[f32]) -> bool + Send>;

/// The part policy for one live segment: it counts the audio of the open
/// part, finds pauses with VAD, keeps the audio to replay, and turns a
/// finished stream into a [`LivePart`].
pub struct Parts {
    policy: PartPolicy,
    /// Voice decision for one frame of [`FRAME_SAMPLES`] samples.
    classify: Classifier,
    /// Segment time of the open part's first sample.
    start_ms: u64,
    /// Samples fed into the open part, the replay included.
    samples: u64,
    frame: Vec<f32>,
    silence_ms: u64,
    /// The last `replay_ms` of segment audio.
    recent: VecDeque<f32>,
    /// Audio to feed into the next stream before new audio.
    replay: Vec<f32>,
    /// Words of the open part whose midpoint is before this time repeat
    /// words of the part before it.
    floor_ms: Option<u64>,
    cut: Option<Cut>,
    /// Voiced time of every part so far, for the stall rule (KTD3).
    voiced_ms: u64,
}

impl Parts {
    /// Parts for a segment whose next audio starts at `start_ms`, with pauses
    /// found by the Earshot VAD.
    pub fn new(start_ms: u64, policy: PartPolicy) -> Self {
        let mut vad = EarshotVad::new(THRESHOLD).expect("threshold is in range");
        // A non-finite sample is a corrupt frame, not speech.
        Self::with_classifier(start_ms, policy, move |frame: &[f32]| {
            vad.is_voice(frame).unwrap_or(false)
        })
    }

    /// Parts with a given voice decision for each frame of
    /// [`FRAME_SAMPLES`] samples.
    pub fn with_classifier(
        start_ms: u64,
        policy: PartPolicy,
        classify: impl FnMut(&[f32]) -> bool + Send + 'static,
    ) -> Self {
        Self {
            policy,
            classify: Box::new(classify),
            start_ms,
            samples: 0,
            frame: Vec::with_capacity(FRAME_SAMPLES),
            silence_ms: 0,
            recent: VecDeque::new(),
            replay: Vec::new(),
            floor_ms: None,
            cut: None,
            voiced_ms: 0,
        }
    }

    /// Segment time of the open part's first sample.
    pub fn part_start_ms(&self) -> u64 {
        self.start_ms
    }

    /// The timed words of the open part so far, in milliseconds from the
    /// start of the segment, without the words that repeat the part before
    /// it. The shift, the clamp and the replay filter are the same as in
    /// [`Parts::finish`]. An open part has no cut yet, so no words go to the
    /// next part.
    pub fn open_words(&self, stream: &dyn LiveStream) -> Vec<LiveWord> {
        let end_ms = self.start_ms + self.samples / SAMPLES_PER_MS;
        in_part(stream.open_words(), self.start_ms, end_ms, self.floor_ms)
    }

    /// The voiced time that every part so far has heard, the replay
    /// included. A new part and [`Parts::restart_at`] do not reset it.
    pub fn voiced_ms(&self) -> u64 {
        self.voiced_ms
    }

    /// Feed the next segment audio into `stream`. The first feed after a
    /// cut at the limit feeds the replay first.
    pub fn feed(&mut self, stream: &mut dyn LiveStream, pcm: &[f32]) -> Result<Fed, AsrError> {
        let mut changed = false;
        if !self.replay.is_empty() {
            let replay = std::mem::take(&mut self.replay);
            changed |= self.hear(stream, &replay, false)?;
        }
        changed |= self.hear(stream, pcm, true)?;
        let length_ms = self.samples / SAMPLES_PER_MS;
        let cut = if length_ms >= self.policy.max_ms {
            Some(Cut::Limit)
        } else if length_ms >= self.policy.min_ms && self.silence_ms >= self.policy.pause_ms {
            Some(Cut::Pause)
        } else {
            None
        };
        if cut.is_some() {
            self.cut = cut;
        }
        Ok(Fed { changed, cut })
    }

    fn hear(
        &mut self,
        stream: &mut dyn LiveStream,
        pcm: &[f32],
        new_audio: bool,
    ) -> Result<bool, AsrError> {
        let changed = stream.feed(pcm)?;
        self.samples += pcm.len() as u64;
        if new_audio {
            self.recent.extend(pcm);
            let keep = (self.policy.replay_ms * SAMPLES_PER_MS) as usize;
            let excess = self.recent.len().saturating_sub(keep);
            self.recent.drain(..excess);
        }
        let mut rest = pcm;
        while !rest.is_empty() {
            let take = (FRAME_SAMPLES - self.frame.len()).min(rest.len());
            self.frame.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            if self.frame.len() == FRAME_SAMPLES {
                self.silence_ms = if (self.classify)(&self.frame) {
                    self.voiced_ms += FRAME_MS;
                    0
                } else {
                    self.silence_ms + FRAME_MS
                };
                self.frame.clear();
            }
        }
        Ok(changed)
    }

    /// Finish the open part: end `stream`'s input and build the part's words.
    /// The next part starts where this one ends, or in the replay after a cut
    /// at the limit. When the stream fails, the part is lost and the error is
    /// returned; the next part is ready all the same.
    pub fn finish(&mut self, stream: &mut dyn LiveStream) -> Result<LivePart, AsrError> {
        let start_ms = self.start_ms;
        let end_ms = start_ms + self.samples / SAMPLES_PER_MS;
        let floor_ms = self.floor_ms.take();
        // After a cut at the limit, this part keeps the words that start in
        // the first half of the replay, and the next part the rest.
        let boundary_ms = match self.cut.take() {
            Some(Cut::Limit) => {
                let replay_ms = self.recent.len() as u64 / SAMPLES_PER_MS;
                self.replay = self.recent.iter().copied().collect();
                self.start_ms = end_ms - replay_ms;
                Some(end_ms - replay_ms / 2)
            }
            Some(Cut::Pause) | None => {
                self.start_ms = end_ms;
                None
            }
        };
        self.samples = 0;
        self.silence_ms = 0;
        self.frame.clear();

        let tokens = stream.finish()?;
        let mut words = in_part(words_from_tokens(&tokens, 0), start_ms, end_ms, floor_ms);
        if let Some(boundary) = boundary_ms {
            words.retain(|w| w.start_ms < boundary);
            self.floor_ms = Some(words.last().map_or(boundary, |w| w.end_ms));
        }
        Ok(LivePart {
            start_ms,
            end_ms,
            words,
        })
    }

    /// Start the next part at `start_ms`, after a gap: dropped frames or a
    /// stream failure. Nothing is replayed across a gap.
    pub fn restart_at(&mut self, start_ms: u64) {
        self.start_ms = start_ms;
        self.samples = 0;
        self.silence_ms = 0;
        self.frame.clear();
        self.recent.clear();
        self.replay.clear();
        self.floor_ms = None;
        self.cut = None;
    }
}

/// Move `words` from stream time to segment time for a part that starts at
/// `start_ms` and ends at `end_ms`, keep them in that span, and remove the
/// words whose midpoint is before `floor_ms`.
fn in_part(
    mut words: Vec<LiveWord>,
    start_ms: u64,
    end_ms: u64,
    floor_ms: Option<u64>,
) -> Vec<LiveWord> {
    for word in &mut words {
        word.start_ms = (start_ms + word.start_ms).min(end_ms);
        word.end_ms = (start_ms + word.end_ms).clamp(word.start_ms, end_ms);
    }
    if let Some(floor) = floor_ms {
        words.retain(|w| (w.start_ms + w.end_ms) / 2 >= floor);
    }
    words
}

/// Stream `pcm` through `recognizer` in feeds of `chunk` samples, one stream
/// for each part. `None` when the model does not stream. The live worker
/// runs the same loop over its frames.
pub fn stream_all(
    recognizer: &mut dyn Recognizer,
    parts: &mut Parts,
    pcm: &[f32],
    chunk: usize,
) -> Result<Option<Vec<LivePart>>, AsrError> {
    let Some(mut stream) = recognizer.start_stream()? else {
        return Ok(None);
    };
    let mut finished = Vec::new();
    for piece in pcm.chunks(chunk) {
        if parts.feed(&mut *stream, piece)?.cut.is_some() {
            finished.push(parts.finish(&mut *stream)?);
            drop(stream);
            stream = recognizer
                .start_stream()?
                .ok_or_else(|| AsrError::Run("the model stopped streaming".into()))?;
        }
    }
    let last = parts.finish(&mut *stream)?;
    if last.end_ms > last.start_ms {
        finished.push(last);
    }
    Ok(Some(finished))
}
