//! The summarize stage (KTD16, R17, R18, R19, R24b). The transcript is
//! rebuilt from the join's inputs, the context budget resolved, and the
//! summary produced in one call when it fits, which is the normal case, or
//! in parts merged layer by layer when it does not. Every call's result is
//! checkpointed, so a quit resumes at the first missing one; a rejection
//! showing the model's context is smaller than assumed shrinks the budget,
//! learns it for the endpoint, and re-chunks that layer.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use session_core::folder::{Summary, SummaryMeta, TranscriptLine};
use session_core::{Anchor, Segment, Stage};
use summarize::chunk::{self, Chunk, Note, Turn};
use summarize::client::Message;
use summarize::{Budget, Client, Endpoint, SummarizeError, context, prompts, tokens};

use crate::diarize::speakers;
use crate::transcribe::results;
use crate::{Control, SharedStore, StageError, checkpoint, lock};

pub const SUMMARY_FILE: &str = "summary.md";
const PLAN: &str = "plan";
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

/// The transcript's turns, rebuilt exactly as the join rendered them.
pub fn transcript_turns(
    store: &SharedStore,
    session_id: &str,
    segments: &[Segment],
) -> Result<Vec<Turn>, StageError> {
    let transcript = crate::join::build(
        segments,
        &results(store, session_id)?,
        &speakers(store, session_id)?,
    );
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

fn session_notes(store: &SharedStore, session_id: &str) -> Result<Vec<Note>, StageError> {
    Ok(lock(store)
        .notes(session_id)?
        .into_iter()
        .filter(|n| !n.text.trim().is_empty())
        .map(|n| Note {
            position_ms: match n.anchor {
                Anchor::At { position_ms, .. } => Some(position_ms),
                Anchor::Unanchored => None,
            },
            text: n.text,
        })
        .collect())
}

/// Forget a summary so the stage runs again from the transcript (R19).
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
    learn: &dyn Fn(u32),
    control: &dyn Control,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<(), StageError> {
    let summary = produce(
        store, session_id, segments, endpoint, learn, control, progress,
    )?;
    let path = session_dir.join(SUMMARY_FILE);
    let temp = session_dir.join(format!("{SUMMARY_FILE}.tmp"));
    std::fs::write(&temp, summary.render())?;
    crate::sync_file(&temp)?;
    std::fs::rename(&temp, &path)?;
    checkpoint::mark_complete(store, session_id, Stage::Summarize, segments.len())
}

/// Summarize the session from its checkpoints, resuming any calls already
/// made. Writes nothing but checkpoints.
pub fn produce(
    store: &SharedStore,
    session_id: &str,
    segments: &[Segment],
    endpoint: SummaryEndpoint,
    learn: &dyn Fn(u32),
    control: &dyn Control,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<Summary, StageError> {
    let turns = transcript_turns(store, session_id, segments)?;
    let notes = session_notes(store, session_id)?;
    let (anchored, general): (Vec<Note>, Vec<Note>) =
        notes.into_iter().partition(|n| n.position_ms.is_some());
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
    let text = if turns.is_empty() {
        // Nothing was said (AE16): there is nothing to send.
        String::from("Nothing was transcribed in this session.")
    } else {
        loop {
            let mut calls = Calls {
                store,
                session_id,
                client: &client,
                runtime: &runtime,
                control,
                general: &general,
                progress: &mut *progress,
            };
            match calls.summarize(&turns, &anchored, &plan) {
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

    Ok(Summary {
        meta: SummaryMeta::Complete {
            endpoint_host: client.host(),
            model: endpoint.model.clone(),
        },
        body: text,
    })
}

fn forget_layers_from(
    store: &SharedStore,
    session_id: &str,
    layer: usize,
) -> Result<(), StageError> {
    let guard = lock(store);
    for (key, _) in guard.checkpoints(session_id, Stage::Summarize)? {
        let at = key
            .strip_prefix('l')
            .and_then(|k| k.get(..2))
            .and_then(|l| l.parse::<usize>().ok());
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
    general: &'a [Note],
    progress: &'a mut dyn FnMut(usize, usize),
}

impl Calls<'_> {
    fn summarize(
        &mut self,
        turns: &[Turn],
        anchored: &[Note],
        plan: &Plan,
    ) -> Result<String, Outcome> {
        let room = plan.room(0);
        let whole = prompts::whole(turns, anchored, self.general);
        if tokens::with_margin(cost(&whole)) + room.reserve as usize <= room.limit as usize {
            return self.call(0, 0, &whole, room, 1);
        }

        // Too long for one call: parts, then merges until one remains.
        let allowance = chunk_allowance(room, self.general);
        let chunks: Vec<Chunk> = chunk::chunk(turns, anchored, allowance);
        let total = chunks.len();
        let mut summaries = Vec::with_capacity(total);
        for (i, c) in chunks.iter().enumerate() {
            summaries.push(self.call(
                0,
                i,
                &prompts::part(c, i, total, self.general),
                room,
                total,
            )?);
        }
        let mut layer = 1;
        if summaries.len() == 1 {
            // The margins let one chunk hold what one whole call could not:
            // its notes still need the summary's shape.
            let only = [summaries[0].as_str()];
            return self.call(
                1,
                0,
                &prompts::merge(&only, true, self.general),
                plan.room(1),
                1,
            );
        }
        while summaries.len() > 1 {
            let room = plan.room(layer);
            let allowance = chunk_allowance(room, self.general);
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
                    &prompts::merge(&parts, last, self.general),
                    room,
                    groups.len(),
                )?);
            }
            summaries = next;
            layer += 1;
        }
        Ok(summaries.pop().unwrap_or_default())
    }

    fn call(
        &mut self,
        layer: usize,
        index: usize,
        messages: &[Message],
        room: Room,
        total: usize,
    ) -> Result<String, Outcome> {
        let key = call_key(layer, index);
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

fn chunk_allowance(room: Room, general: &[Note]) -> usize {
    tokens::prompt_allowance(room.limit, room.reserve)
        .saturating_sub(prompts::overhead(general))
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
