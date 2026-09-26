//! The summarize stage (KTD16, R17, R18, R19, R24b). The transcript is
//! read from `transcript.md` (KTD4), the context budget resolved, and the
//! summary produced in one call when it fits, which is the normal case, or
//! in parts merged layer by layer when it does not. Every call's result is
//! checkpointed, so a quit resumes at the first missing one; a rejection
//! showing the model's context is smaller than assumed shrinks the budget,
//! learns it for the endpoint, and re-chunks that layer.
//!
//! The notes document goes whole with every call (KTD6), as it was when the
//! run started (R36). After the final summary, one more call proposes a
//! title, unless the user set one (KTD7).
//!
//! A run takes a snapshot of the active summary prompt before its first
//! call and keeps it to the end, also after a resume (KTD4). The snapshot's
//! text goes into the final call, and `summary.md` records its name.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use session_core::folder::{Summary, SummaryMeta, Transcript, TranscriptLine};
use session_core::{Segment, Stage, TitleSource};
use summarize::chunk::{self, Chunk, Turn};
use summarize::client::Message;
use summarize::{Budget, Client, Endpoint, SummarizeError, context, prompts, tokens};

use crate::join::TRANSCRIPT_FILE;
use crate::{Control, SharedStore, StageError, checkpoint, lock};

pub use summarize::Prompt;

pub const SUMMARY_FILE: &str = "summary.md";
/// The layer structure that the run committed to, a `Plan`.
pub const PLAN: &str = "plan";
/// The snapshot of the summary prompt that the run uses, a [`Prompt`]
/// (KTD4). It is saved next to the plan, before the first call, and a
/// resumed run uses it in place of the active prompt.
pub const PROMPT: &str = "prompt";
/// The result of the whole-transcript call. The key is not a part note's
/// key, so a run that decides the fit again never uses one result for the
/// other (KTD5).
pub const WHOLE: &str = "whole";
/// The error when the fixed parts of the final call and the prompt text
/// leave too little of the model's context for the transcript (KTD8).
const PROMPT_TOO_LONG: &str = "the summary prompt is too long for this model's context; \
shorten the prompt in Settings or choose a model with a larger context";
/// The proposed title, a string, once the title call has run. An empty
/// string means that the reply held no title.
pub const TITLE: &str = "title";
/// How far a context rejection shrinks the budget, and the floor below
/// which a model is too small to summarize with.
const SHRINK: f64 = 0.7;
const MIN_LIMIT: u32 = 2_048;
const MAX_SHRINKS: usize = 6;
const CANCEL_POLL: Duration = Duration::from_millis(250);

/// Where the summary goes, from the app's settings and secret store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryEndpoint {
    pub base_url: String,
    pub model: String,
    pub credential: Option<String>,
    /// The user's context setting, which overrides the probe.
    pub context_limit: Option<u32>,
    /// A limit an earlier rejection taught for this endpoint and model.
    pub learned_limit: Option<u32>,
}

/// The layer structure a run committed to, so a resumed run reuses the
/// checkpoints it made rather than re-chunking under a different budget.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Plan {
    budget: Budget,
    /// Per layer, what it was built under; later layers inherit the last.
    layers: Vec<Room>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Room {
    limit: u32,
    /// Tokens kept for the reply, and sent as its cap.
    reserve: u32,
}

impl Room {
    fn new(limit: u32) -> Self {
        Self {
            limit,
            reserve: tokens::reply_reserve(limit),
        }
    }
}

impl Plan {
    fn room(&self, layer: usize) -> Room {
        *self
            .layers
            .get(layer)
            .unwrap_or(self.layers.last().expect("a plan has a layer"))
    }

    /// Rebuild `layer` and everything after it under `room`.
    fn replace_from(&mut self, layer: usize, room: Room) {
        self.layers.truncate(layer);
        while self.layers.len() < layer {
            let last = *self.layers.last().expect("a plan has a layer");
            self.layers.push(last);
        }
        self.layers.push(room);
    }
}

fn call_key(layer: usize, index: usize) -> String {
    format!("l{layer:02}-{index:05}")
}

/// The turns of the `transcript.md` in `folder` (KTD4). The file holds the
/// final text, so the turns are the same on every computer, also on one
/// that has no recognition results for the note.
pub fn transcript_turns(folder: &Path) -> Result<Vec<Turn>, StageError> {
    let path = folder.join(TRANSCRIPT_FILE);
    let text = std::fs::read_to_string(&path).map_err(|e| {
        StageError::Failed(format!(
            "the transcript {} cannot be read: {e}",
            path.display()
        ))
    })?;
    let transcript = Transcript::parse(&text).map_err(|e| {
        StageError::Failed(format!(
            "the transcript {} is not valid: {e}",
            path.display()
        ))
    })?;
    Ok(transcript
        .lines
        .into_iter()
        .filter_map(|line| match line {
            TranscriptLine::Turn {
                position_ms,
                speaker,
                text,
            } => Some(Turn {
                position_ms,
                speaker,
                text,
            }),
            _ => None,
        })
        .collect())
}

/// The notes document as it is now. A run reads it once, when it starts,
/// and sends that copy with every call (R36).
fn session_notes(store: &SharedStore, session_id: &str) -> Result<String, StageError> {
    Ok(lock(store).notes(session_id)?)
}

/// The title call's reply as a title: its first line with text, with no
/// heading marks, no `Title:` label, no quotes around it and no full stop
/// at the end, cut to the length that a folder name keeps (KTD7).
pub fn clean_title(reply: &str) -> String {
    const MARKS: &[char] = &['"', '\'', '`', '*', '“', '”', '‘', '’', '«', '»'];
    let line = reply
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let mut title = line.trim_start_matches('#').trim();
    if title
        .get(..6)
        .is_some_and(|label| label.eq_ignore_ascii_case("title:"))
    {
        title = title[6..].trim();
    }
    loop {
        let next = title.trim_end_matches('.').trim_matches(MARKS).trim();
        if next == title {
            break;
        }
        title = next;
    }
    let words = title.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut: String = words
        .chars()
        .take(session_save::naming::MAX_TITLE_CHARS)
        .collect();
    cut.trim_end().to_string()
}

/// Forget a summary and its title proposal, so the stage runs again from
/// the transcript (R19, R31).
pub fn clear(store: &SharedStore, session_id: &str) -> Result<(), StageError> {
    Ok(lock(store).clear_checkpoints(session_id, Stage::Summarize)?)
}

/// The summarize stage: produce the summary and write it into the working
/// directory, where the save stage picks it up.
#[allow(clippy::too_many_arguments)]
pub fn run(
    store: &SharedStore,
    session_id: &str,
    session_dir: &Path,
    segments: &[Segment],
    endpoint: SummaryEndpoint,
    active: Prompt,
    learn: &dyn Fn(u32),
    control: &dyn Control,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<(), StageError> {
    let summary = produce(
        store,
        session_id,
        session_dir,
        endpoint,
        active,
        learn,
        control,
        progress,
    )?;
    let path = session_dir.join(SUMMARY_FILE);
    let temp = session_dir.join(format!("{SUMMARY_FILE}.tmp"));
    std::fs::write(&temp, summary.render())?;
    crate::sync_file(&temp)?;
    std::fs::rename(&temp, &path)?;
    checkpoint::mark_complete(store, session_id, Stage::Summarize, segments.len())
}

/// Summarize the `transcript.md` in `folder`, resuming any calls already
/// made. Writes nothing but checkpoints. `active` is the active summary
/// prompt, which the run uses only when it saved no snapshot before.
#[allow(clippy::too_many_arguments)]
pub fn produce(
    store: &SharedStore,
    session_id: &str,
    folder: &Path,
    endpoint: SummaryEndpoint,
    active: Prompt,
    learn: &dyn Fn(u32),
    control: &dyn Control,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<Summary, StageError> {
    let turns = transcript_turns(folder)?;
    let notes = session_notes(store, session_id)?;
    let client = Client::new(Endpoint {
        base_url: endpoint.base_url.clone(),
        model: endpoint.model.clone(),
        credential: endpoint.credential.clone(),
    })
    .map_err(|e| StageError::Failed(e.to_string()))?;
    if client.crosses_network_unencrypted() {
        log::warn!(
            "summarizing {session_id} over plain HTTP to {}: the transcript crosses the network unencrypted",
            client.host()
        );
    }
    let runtime = Runtime::new()?;

    let prompt = match checkpoint::get::<Prompt>(store, session_id, Stage::Summarize, PROMPT)? {
        Some(saved) => saved,
        None => {
            // A run from before prompts were saved can have a plan and
            // cached calls made with another prompt. Remove them, so that
            // no result goes out under the name of the active prompt. The
            // stage's completion and failure records stay.
            forget_calls(store, session_id)?;
            checkpoint::put(store, session_id, Stage::Summarize, PROMPT, &active)?;
            active
        }
    };

    let mut plan = match checkpoint::get::<Plan>(store, session_id, Stage::Summarize, PLAN)? {
        Some(plan) => plan,
        None => {
            control.proceed()?;
            let probed = if endpoint.context_limit.is_some() {
                None
            } else {
                runtime.block_on(context::probe(&client), control)?
            };
            let budget = context::resolve(endpoint.context_limit, probed, endpoint.learned_limit);
            log::info!(
                "summarizing {session_id} with {} on {}: budget {} tokens ({:?})",
                endpoint.model,
                client.host(),
                budget.limit,
                budget.source
            );
            let plan = Plan {
                budget,
                layers: vec![Room::new(budget.limit)],
            };
            checkpoint::put(store, session_id, Stage::Summarize, PLAN, &plan)?;
            plan
        }
    };

    let mut shrinks = 0;
    let mut calls = Calls {
        store,
        session_id,
        client: &client,
        runtime: &runtime,
        control,
        notes: &notes,
        text: &prompt.text,
        progress: &mut *progress,
    };
    let text = if turns.is_empty() {
        // Nothing was said (AE16): there is nothing to send.
        String::from("Nothing was transcribed in this note.")
    } else {
        loop {
            match calls.summarize(&turns, &plan) {
                Ok(text) => break text,
                Err(Pressure { layer, room }) if shrinks < MAX_SHRINKS => {
                    shrinks += 1;
                    let smaller = ((room.limit as f64 * SHRINK) as u32).max(MIN_LIMIT);
                    if smaller >= room.limit {
                        return Err(StageError::Failed(format!(
                            "the model's context is too small to summarize with (under {MIN_LIMIT} tokens)"
                        )));
                    }
                    log::warn!(
                        "{} rejected a layer-{layer} prompt under {} tokens; retrying at {smaller}",
                        client.host(),
                        room.limit
                    );
                    learn(smaller);
                    forget_layers_from(store, session_id, layer)?;
                    plan.replace_from(
                        layer,
                        Room {
                            limit: smaller,
                            reserve: room.reserve.min(smaller / 2),
                        },
                    );
                    checkpoint::put(store, session_id, Stage::Summarize, PLAN, &plan)?;
                }
                Err(CutOff { layer, room }) if shrinks < MAX_SHRINKS => {
                    // The prompt fit and the reply did not: give the reply
                    // more of the same context and send less with it.
                    shrinks += 1;
                    let reserve = (room.reserve * 2).min(room.limit / 2);
                    if reserve <= room.reserve {
                        return Err(StageError::Failed(
                            "the model's replies do not fit in its context; raise its context length or choose another model".into(),
                        ));
                    }
                    log::warn!(
                        "{} cut off a layer-{layer} reply at {} tokens; retrying with {reserve}",
                        client.host(),
                        room.reserve
                    );
                    forget_layers_from(store, session_id, layer)?;
                    plan.replace_from(
                        layer,
                        Room {
                            limit: room.limit,
                            reserve,
                        },
                    );
                    checkpoint::put(store, session_id, Stage::Summarize, PLAN, &plan)?;
                }
                Err(Pressure { .. } | CutOff { .. }) => {
                    return Err(StageError::Failed(
                        "the endpoint kept rejecting prompts or cutting off replies".into(),
                    ));
                }
                Err(Stop(e)) => return Err(e),
            }
        }
    };
    if !turns.is_empty() {
        let room = *plan.layers.last().expect("a plan has a layer");
        calls.propose_title(&text, room)?;
    }

    Ok(Summary {
        meta: SummaryMeta::Complete {
            endpoint_host: client.host(),
            model: endpoint.model.clone(),
            // The name comes from the snapshot, so it always matches the
            // text that was sent (R11). With nothing said, no prompt made
            // the summary.
            prompt_name: (!turns.is_empty()).then(|| prompt.name.clone()),
        },
        body: text,
    })
}

/// Remove the plan, every cached call and the title proposal.
fn forget_calls(store: &SharedStore, session_id: &str) -> Result<(), StageError> {
    forget_layers_from(store, session_id, 0)?;
    let guard = lock(store);
    guard.delete_checkpoint(session_id, Stage::Summarize, PLAN)?;
    guard.delete_checkpoint(session_id, Stage::Summarize, TITLE)?;
    Ok(())
}

fn forget_layers_from(
    store: &SharedStore,
    session_id: &str,
    layer: usize,
) -> Result<(), StageError> {
    let guard = lock(store);
    for (key, _) in guard.checkpoints(session_id, Stage::Summarize)? {
        // The whole call is in layer 0.
        let at = if key == WHOLE {
            Some(0)
        } else {
            key.strip_prefix('l')
                .and_then(|k| k.get(..2))
                .and_then(|l| l.parse::<usize>().ok())
        };
        if at.is_some_and(|l| l >= layer) {
            guard.delete_checkpoint(session_id, Stage::Summarize, &key)?;
        }
    }
    Ok(())
}

enum Outcome {
    /// The prompt at `layer`, built under `room`, was too big for the model.
    Pressure {
        layer: usize,
        room: Room,
    },
    /// The reply at `layer` ran out of its reserve.
    CutOff {
        layer: usize,
        room: Room,
    },
    Stop(StageError),
}
use Outcome::{CutOff, Pressure, Stop};

impl From<StageError> for Outcome {
    fn from(e: StageError) -> Self {
        Stop(e)
    }
}

struct Calls<'a> {
    store: &'a SharedStore,
    session_id: &'a str,
    client: &'a Client,
    runtime: &'a Runtime,
    control: &'a dyn Control,
    /// The notes document as it was when the run started (R36).
    notes: &'a str,
    /// The text of the run's prompt snapshot (KTD4).
    text: &'a str,
    progress: &'a mut dyn FnMut(usize, usize),
}

impl Calls<'_> {
    fn summarize(&mut self, turns: &[Turn], plan: &Plan) -> Result<String, Outcome> {
        let text = self.text;
        let room = plan.room(0);
        // The final call's fixed parts and the prompt text must leave at
        // least half of the prompt allowance for the transcript (KTD8).
        if prompts::overhead("", text) * 2 > tokens::prompt_allowance(room.limit, room.reserve) {
            // Remove the plan, so that the next run takes the budget from
            // the endpoint again. A model with a larger context can then
            // run the same prompt.
            lock(self.store)
                .delete_checkpoint(self.session_id, Stage::Summarize, PLAN)
                .map_err(StageError::from)?;
            return Err(Stop(StageError::Failed(PROMPT_TOO_LONG.into())));
        }
        let notes = fit_notes(room, self.notes, text);
        let whole = prompts::whole(turns, &notes, text);
        if tokens::with_margin(cost(&whole)) + room.reserve as usize <= room.limit as usize {
            return self.call_as(WHOLE.into(), 0, 0, &whole, room, 1);
        }

        // Too long for one call: parts, then merges until one remains.
        let allowance = chunk_allowance(room, &notes, text);
        let chunks: Vec<Chunk> = chunk::chunk(turns, allowance);
        let total = chunks.len();
        let mut summaries = Vec::with_capacity(total);
        for (i, c) in chunks.iter().enumerate() {
            summaries.push(self.call(0, i, &prompts::part(c, i, total, &notes), room, total)?);
        }
        let mut layer = 1;
        if summaries.len() == 1 {
            // The margins let one chunk hold what one whole call could not:
            // its notes still need the summary's shape.
            let only = [summaries[0].as_str()];
            let room = plan.room(1);
            return self.call(
                1,
                0,
                &prompts::merge(&only, true, &fit_notes(room, self.notes, text), text),
                room,
                1,
            );
        }
        while summaries.len() > 1 {
            let room = plan.room(layer);
            let notes = fit_notes(room, self.notes, text);
            let allowance = chunk_allowance(room, &notes, text);
            let costs: Vec<usize> = summaries.iter().map(|s| tokens::estimate(s) + 8).collect();
            let groups = chunk::merge_groups(&costs, allowance);
            let last = groups.len() == 1;
            let mut next = Vec::with_capacity(groups.len());
            for (i, g) in groups.iter().enumerate() {
                if g.len() == 1 {
                    next.push(summaries[g.start].clone());
                    continue;
                }
                let parts: Vec<&str> = summaries[g.clone()].iter().map(String::as_str).collect();
                next.push(self.call(
                    layer,
                    i,
                    &prompts::merge(&parts, last, &notes, text),
                    room,
                    groups.len(),
                )?);
            }
            summaries = next;
            layer += 1;
        }
        Ok(summaries.pop().unwrap_or_default())
    }

    /// Propose a title from the final summary and the notes (KTD7), unless
    /// the user set a title. The proposal never replaces a title that the
    /// user sets during the call (R31). A failed call leaves the session
    /// with the title it has, and the summary still completes. Only a
    /// cancel stops the stage.
    fn propose_title(&mut self, summary: &str, room: Room) -> Result<(), StageError> {
        if checkpoint::get::<String>(self.store, self.session_id, Stage::Summarize, TITLE)?
            .is_some()
        {
            return Ok(());
        }
        let (user_title, again) = {
            let guard = lock(self.store);
            (
                guard
                    .title(self.session_id)?
                    .is_some_and(|t| t.source == TitleSource::User),
                guard
                    .transcribe_again_marks(self.session_id)?
                    .model
                    .is_some(),
            )
        };
        // A Transcribe Again run keeps the title and so the folder name
        // (R16, KTD14).
        if user_title || again {
            return Ok(());
        }
        self.control.proceed()?;
        let allowance = tokens::prompt_allowance(room.limit, room.reserve)
            .saturating_sub(tokens::estimate(summary));
        let messages = prompts::title(summary, &prompts::fit_notes(self.notes, allowance, ""));
        let reply = self
            .runtime
            .block_on(self.client.complete(&messages, room.reserve), self.control)?;
        let title = match reply {
            Ok(completion) => clean_title(&completion.text),
            Err(e) => {
                log::warn!(
                    "proposing a title for {} failed: {e}; the session keeps the title it has",
                    self.session_id
                );
                return Ok(());
            }
        };
        // The title and its checkpoint change together, so a resumed run
        // does not propose again over an edit the user made after.
        let guard = lock(self.store);
        guard.set_title(self.session_id, &title, TitleSource::Proposed)?;
        guard.put_checkpoint(
            self.session_id,
            Stage::Summarize,
            TITLE,
            &serde_json::to_string(&title)?,
        )?;
        Ok(())
    }

    fn call(
        &mut self,
        layer: usize,
        index: usize,
        messages: &[Message],
        room: Room,
        total: usize,
    ) -> Result<String, Outcome> {
        self.call_as(call_key(layer, index), layer, index, messages, room, total)
    }

    /// One call, with its result saved under `key`.
    fn call_as(
        &mut self,
        key: String,
        layer: usize,
        index: usize,
        messages: &[Message],
        room: Room,
        total: usize,
    ) -> Result<String, Outcome> {
        if let Some(done) =
            checkpoint::get::<String>(self.store, self.session_id, Stage::Summarize, &key)?
        {
            return Ok(done);
        }
        self.control.proceed()?;
        let reply = self
            .runtime
            .block_on(self.client.complete(messages, room.reserve), self.control)?;
        match reply {
            Ok(completion) => {
                checkpoint::put(
                    self.store,
                    self.session_id,
                    Stage::Summarize,
                    &key,
                    &completion.text,
                )?;
                (self.progress)(index + 1, total);
                Ok(completion.text)
            }
            Err(e) if e.is_context_pressure() => Err(Pressure { layer, room }),
            Err(SummarizeError::ReplyCutOff) => Err(CutOff { layer, room }),
            Err(e) => Err(Stop(StageError::Failed(describe(layer, index, total, &e)))),
        }
    }
}

fn describe(layer: usize, index: usize, total: usize, e: &SummarizeError) -> String {
    if layer == 0 && total > 1 {
        format!("summarizing part {} of {total}: {e}", index + 1)
    } else if layer == 0 {
        format!("summarizing: {e}")
    } else {
        format!("combining the summaries (layer {layer}): {e}")
    }
}

fn cost(messages: &[Message]) -> usize {
    tokens::estimate_messages(messages.iter().map(|m| m.content.as_str()))
}

/// The notes document, cut when it would take too much of what `room`
/// lets a prompt use with the prompt `text`.
fn fit_notes(room: Room, notes: &str, text: &str) -> String {
    prompts::fit_notes(
        notes,
        tokens::prompt_allowance(room.limit, room.reserve),
        text,
    )
}

/// What is left for the transcript or the summaries in one call, after the
/// prompt's fixed parts, the notes and the prompt `text`.
fn chunk_allowance(room: Room, notes: &str, text: &str) -> usize {
    tokens::prompt_allowance(room.limit, room.reserve)
        .saturating_sub(prompts::overhead(notes, text))
        .max(1)
}

/// Stages run on the blocking pool; the client is async. Inside the app's
/// runtime this borrows its handle, elsewhere it builds one.
struct Runtime {
    handle: tokio::runtime::Handle,
    _owned: Option<tokio::runtime::Runtime>,
}

impl Runtime {
    fn new() -> Result<Self, StageError> {
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            return Ok(Self {
                handle,
                _owned: None,
            });
        }
        let owned = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        Ok(Self {
            handle: owned.handle().clone(),
            _owned: Some(owned),
        })
    }

    /// Run `work` to completion unless the stage is cancelled first: a
    /// resume mid-call discards the call rather than waiting it out (R6b).
    fn block_on<T>(
        &self,
        work: impl std::future::Future<Output = T>,
        control: &dyn Control,
    ) -> Result<T, StageError> {
        self.handle.block_on(async {
            tokio::pin!(work);
            loop {
                tokio::select! {
                    out = &mut work => return Ok(out),
                    _ = tokio::time::sleep(CANCEL_POLL) => {
                        if control.is_cancelled() {
                            return Err(StageError::Cancelled);
                        }
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::clean_title;

    #[test]
    fn a_reply_is_cleaned_to_a_plain_title() {
        assert_eq!(clean_title("  \"Budget Review.\"  "), "Budget Review");
        assert_eq!(clean_title("\"Budget Review\"."), "Budget Review");
        assert_eq!(clean_title("\n# Title: **Q3 plan**\nmore"), "Q3 plan");
        assert_eq!(clean_title("“Hiring  and\tbudget”"), "Hiring and budget");
        assert_eq!(clean_title("Plan v2.0"), "Plan v2.0");
        assert_eq!(clean_title(" \n \"\" "), "");
    }
}
