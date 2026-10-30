//! The summarize stage (KTD16, R17, R18, R19, R24b). The transcript is
//! read from `transcript.md` (KTD4), the context budget resolved, and the
//! summary produced in one call when it fits, which is the normal case, or
//! in parts merged layer by layer when it does not. Every call's result is
//! checkpointed, so a quit resumes at the first missing one. A rejection
//! that shows the model's context is smaller than the budget stops the
//! run: only the user changes the context limit.
//!
//! The notes document goes whole with every call (KTD6), as it was when the
//! run started (R36). The note details (the shown type, the tags and a
//! title that the user typed) go with every call in the same way. After the
//! final summary, the stage stores the title turn as a pending title
//! request, unless the user set a title, and sends no title call (KTD3).
//! The title job sends the request after the save.
//!
//! A run takes a snapshot of the active summary prompt before its first
//! call and keeps it to the end, also after a resume (KTD4). The snapshot's
//! text goes into the final call, and `summary.md` records its name.

use std::path::Path;
use std::time::Duration;

use library::scan::{READ_TIMEOUT, Read, read_timed};
use serde::{Deserialize, Serialize};
use session_core::folder::{Summary, SummaryMeta, Transcript, TranscriptLine};
use session_core::{Segment, SessionRecord, Stage, TitleSource};
use summarize::chunk::{self, Chunk, Line, Turn};
use summarize::client::{Label, Message, RequestKind};
use summarize::prompts::{Details, UserText};
use summarize::provider::ThinkingLevel;
use summarize::{
    Budget, BudgetSource, Client, Endpoint, Provider, SummarizeError, context, prompts, tokens,
};

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
/// leave too little of the model's context for the transcript (KTD8). The
/// prompt text and the Background both count (KTD11).
const PROMPT_TOO_LONG: &str = "the summary prompt is too long for this model's context; \
shorten the prompt or the Background in Settings, or choose a model with a larger context";
/// The proposed title, a string, once the title turn has run. An empty
/// string means that the reply held no title.
pub const TITLE: &str = "title";
/// The pending title turn, a [`TitleRequest`] (KTD3). The Summarize stage
/// stores it after the final call, and the title job sends it and deletes
/// it. It is a row in the store, so it stays after Save removes the working
/// folder.
pub const TITLE_REQUEST: &str = "title-request";
/// The fingerprint of the transcript and the notes that the cached calls
/// were made from, a string. A run whose input has another fingerprint
/// makes every call again.
pub const INPUT: &str = "input";
/// The data of the summary that the stage wrote, a [`SummaryMeta`]. The
/// save writes it into `longhand.json`, because `summary.md` has no
/// frontmatter (R1).
pub const META: &str = "meta";
/// Ollama context sizes are multiples of this number of tokens (KTD7).
const CONTEXT_STEP: usize = 2_048;
/// The smallest Ollama context size, unless the context limit is smaller
/// (KTD7).
const SMALLEST_CONTEXT: u32 = 4_096;
/// How many times a cut-off reply gets more room.
const MAX_SHRINKS: usize = 6;
const CANCEL_POLL: Duration = Duration::from_millis(250);

/// Where the summary goes, from the app's settings and secret store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryEndpoint {
    /// The provider, which selects the request route (KTD2). A job keeps
    /// the provider that was in effect when it started (R25).
    pub provider: Provider,
    /// The address of the provider. An empty address selects the default
    /// address of the provider.
    pub base_url: String,
    pub model: String,
    pub credential: Option<String>,
    /// The user's context setting, which overrides the probe.
    pub context_limit: Option<u32>,
    /// The Thinking level that the requests use (R5, R6).
    pub thinking: ThinkingLevel,
    /// Whether the user chose `thinking`. `false` means that it is the
    /// default of the provider kind (KTD7).
    pub thinking_chosen: bool,
    /// How many requests go to the provider at the same time (R12).
    pub requests_at_once: u32,
    /// The waits before each new try of a request that could not reach the
    /// endpoint ([`summarize::client::RETRY_WAITS`] in the app).
    pub retry_waits: Vec<Duration>,
}

/// The layer structure a run committed to, so a resumed run reuses the
/// checkpoints it made rather than re-chunking under a different budget.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Plan {
    budget: Budget,
    /// Per layer, what it was built under; later layers inherit the last.
    layers: Vec<Room>,
    /// The provider, the address and the model that made the plan's calls
    /// (KTD11). A plan from an older version has none.
    #[serde(default)]
    source: Option<Source>,
}

/// Where the calls of a plan went: the provider, the full address (scheme,
/// host, port and path) and the model (KTD11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Source {
    provider: String,
    address: String,
    model: String,
}

impl Source {
    /// The source of the calls that `client` sends.
    fn of(client: &Client) -> Self {
        let endpoint = client.endpoint();
        Self {
            provider: endpoint.provider.name().to_string(),
            address: endpoint
                .address()
                .unwrap_or_default()
                .trim_end_matches('/')
                .to_string(),
            model: endpoint.model.clone(),
        }
    }
}

/// The Ollama context size for a request that needs `need` tokens, the
/// prompt estimate and the reply reserve (KTD7). The size is a multiple of
/// [`CONTEXT_STEP`], at least [`SMALLEST_CONTEXT`], and never more than
/// `limit`, the context limit.
fn context_size(need: usize, limit: u32) -> u32 {
    let rounded = u32::try_from(need.div_ceil(CONTEXT_STEP) * CONTEXT_STEP).unwrap_or(u32::MAX);
    rounded.max(SMALLEST_CONTEXT).min(limit)
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
    /// True when the plan's calls went to `source` and the context limit
    /// setting `user` still gives this plan's budget. A plan with another
    /// provider, address or model, or with no source, is made again
    /// (KTD11). A set limit keeps a plan under the same limit, whatever gave
    /// it, so its calls are not sent again. With no set limit, a plan under
    /// a set limit, under a learned limit of an older version, or under an
    /// older default is made again.
    fn agrees_with(&self, user: Option<u32>, source: &Source) -> bool {
        if self.source.as_ref() != Some(source) {
            return false;
        }
        match (user, self.budget.source) {
            (Some(limit), _) => self.budget.limit == limit,
            (None, BudgetSource::User | BudgetSource::Learned) => false,
            (None, BudgetSource::Default) => self.budget.limit == context::DEFAULT_LIMIT,
            (None, _) => true,
        }
    }

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

/// The title turn that the Summarize stage stores for the title job: the
/// whole request (KTD1) and its reply cap (KTD12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TitleRequest {
    pub messages: Vec<Message>,
    /// The reply cap in tokens. It is never less than
    /// [`prompts::TITLE_REPLY_MIN`].
    pub cap: u32,
    /// The context limit of the final call, which limits the Ollama context
    /// size of the title turn (KTD7). A request from an older version has
    /// none.
    pub limit: Option<u32>,
}

/// The stored form of a [`TitleRequest`]. A [`Message`] has a fixed role
/// text, so the read maps each role to it.
#[derive(Deserialize)]
struct StoredTitleRequest {
    messages: Vec<StoredMessage>,
    cap: u32,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Deserialize)]
struct StoredMessage {
    role: String,
    content: String,
}

/// The pending title request of `note_id`, or `None` when no title turn
/// is due.
pub fn pending_title_request(
    store: &SharedStore,
    note_id: &str,
) -> Result<Option<TitleRequest>, StageError> {
    let Some(stored) =
        checkpoint::get::<StoredTitleRequest>(store, note_id, Stage::Summarize, TITLE_REQUEST)?
    else {
        return Ok(None);
    };
    let messages = stored
        .messages
        .into_iter()
        .map(|m| {
            let role = match m.role.as_str() {
                "system" => "system",
                "user" => "user",
                "assistant" => "assistant",
                other => {
                    return Err(StageError::Failed(format!(
                        "the title request of {note_id} has a message with the unknown role {other:?}"
                    )));
                }
            };
            Ok(Message {
                role,
                content: m.content,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(Some(TitleRequest {
        messages,
        cap: stored.cap,
        limit: stored.limit,
    }))
}

/// Delete the pending title request of `note_id`. A session with no
/// request is not an error.
pub fn delete_title_request(store: &SharedStore, note_id: &str) -> Result<(), StageError> {
    Ok(lock(store).delete_checkpoint(note_id, Stage::Summarize, TITLE_REQUEST)?)
}

/// Send the title turn `request` to `endpoint` with its reply cap, and
/// clean the reply with [`clean_title`] (KTD13). The inner error is a
/// failed turn or a reply that was cut off: the note then keeps its title
/// (R14). Only a cancel is an outer error. `queued` is the time that the
/// job waited for its turn in the summary queue, for the request log.
pub fn send_title_request(
    note_id: &str,
    endpoint: &SummaryEndpoint,
    request: &TitleRequest,
    control: &dyn Control,
    queued: Duration,
) -> Result<Result<String, String>, StageError> {
    // The title job holds a turn in the summary queue, and a title is
    // optional. The request is sent once, with no retry waits, so an
    // endpoint that cannot be reached does not stop the other jobs of the
    // summary queue.
    let client = match Client::new(client_endpoint(endpoint)) {
        Ok(client) => client,
        Err(e) => return Ok(Err(e.to_string())),
    };
    if client.crosses_network_unencrypted() {
        log::warn!(
            "proposing a title for {note_id} over plain HTTP to {}: the transcript crosses the network unencrypted",
            client.host()
        );
    }
    let runtime = Runtime::new()?;
    control.proceed()?;
    let label = Label::note(note_id, RequestKind::Title).queued(queued);
    // The title turn needs its prompt and its reply cap. With no saved
    // limit, the limit of the setting applies, or none.
    let num_ctx = (endpoint.provider == Provider::Ollama).then(|| {
        context_size(
            tokens::with_margin(cost(&request.messages)) + request.cap as usize,
            request.limit.or(endpoint.context_limit).unwrap_or(u32::MAX),
        )
    });
    let reply = runtime.block_on(
        client.complete(&label, &request.messages, request.cap, num_ctx),
        control,
    )?;
    Ok(reply
        .map(|completion| clean_title(&completion.text))
        .map_err(|e| e.to_string()))
}

/// The endpoint of the summarize client for `endpoint`.
fn client_endpoint(endpoint: &SummaryEndpoint) -> Endpoint {
    Endpoint {
        provider: endpoint.provider,
        base_url: endpoint.base_url.clone(),
        model: endpoint.model.clone(),
        credential: endpoint.credential.clone(),
    }
}

/// The step of a summary in parts that a progress report is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryStep {
    /// One call for each part of the transcript.
    Parts,
    /// The calls that combine the part summaries in groups, in every layer
    /// before the last.
    Groups,
    /// The last call, which writes the summary from all the summaries.
    Final,
}

impl SummaryStep {
    pub fn name(self) -> &'static str {
        match self {
            Self::Parts => "parts",
            Self::Groups => "groups",
            Self::Final => "final",
        }
    }
}

fn call_key(layer: usize, index: usize) -> String {
    format!("l{layer:02}-{index:05}")
}

/// The bytes of the `transcript.md` in `folder`, or `None` when the file
/// is not there. The read has a timeout: a synced folder can hold a file
/// that does not download.
pub(crate) fn transcript_bytes(folder: &Path) -> Result<Option<Vec<u8>>, StageError> {
    let path = folder.join(TRANSCRIPT_FILE);
    match read_timed(path.clone(), READ_TIMEOUT) {
        Read::Bytes(bytes) => Ok(Some(bytes)),
        Read::Missing => Ok(None),
        Read::Failed(why) => Err(StageError::Failed(format!(
            "the storage folder is not available: the transcript {} cannot be read: {why}",
            path.display()
        ))),
    }
}

/// The text of the `transcript.md` in `folder`.
fn read_transcript(folder: &Path) -> Result<String, StageError> {
    let path = folder.join(TRANSCRIPT_FILE);
    let Some(bytes) = transcript_bytes(folder)? else {
        return Err(StageError::Failed(format!(
            "the transcript {} is not there",
            path.display()
        )));
    };
    String::from_utf8(bytes).map_err(|e| {
        StageError::Failed(format!(
            "the transcript {} cannot be read: {e}",
            path.display()
        ))
    })
}

/// The turns and pause lines of `text`, a `transcript.md`. The summary
/// model sees each pause at its position (R13). A line in free text
/// (KTD7 of the safe agent edits plan) is a turn with no speaker at the
/// position of the turn before it, so the model also gets that text.
fn lines_of(text: &str) -> Vec<Line> {
    let mut position = 0;
    Transcript::parse(text)
        .lines
        .into_iter()
        .filter_map(|line| match line {
            TranscriptLine::Turn {
                position_ms,
                speaker,
                text,
            } => {
                position = position_ms;
                Some(Line::Turn(Turn {
                    position_ms,
                    speaker,
                    text,
                }))
            }
            TranscriptLine::Text { text } => Some(Line::Turn(Turn {
                position_ms: position,
                speaker: None,
                text,
            })),
            TranscriptLine::Pause { duration_ms } => Some(Line::Pause { duration_ms }),
            TranscriptLine::Gap { .. } | TranscriptLine::Silence { .. } => None,
        })
        .collect()
}

/// The fingerprint of the input to the calls: the transcript text, the
/// notes document and the note details. A note with no details has the
/// fingerprint of its transcript and notes only.
fn input_fingerprint(
    transcript: &str,
    notes: &str,
    details: &Details,
) -> Result<String, StageError> {
    let mut bytes = Vec::with_capacity(transcript.len() + notes.len() + 1);
    bytes.extend_from_slice(transcript.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(notes.as_bytes());
    if !details.is_empty() {
        bytes.push(0);
        bytes.extend_from_slice(
            &serde_json::to_vec(details).map_err(|e| StageError::Failed(e.to_string()))?,
        );
    }
    Ok(session_core::folder::fingerprint(&bytes))
}

/// The store row of the note `note_id`. A note that is not in the
/// store is a failure.
pub(crate) fn note_record(store: &SharedStore, note_id: &str) -> Result<SessionRecord, StageError> {
    lock(store)
        .session(note_id)?
        .ok_or_else(|| StageError::Failed(format!("the note {note_id} is not in the store")))
}

/// The note details as they are now: the type that the app shows, which
/// can be a pending type (KTD3), the tags, and the title when the user
/// typed it. A title that processing proposed is not a detail. A run reads
/// them once, when it starts, and sends that copy with every call.
fn note_details(store: &SharedStore, note_id: &str) -> Result<Details, StageError> {
    let record = note_record(store, note_id)?;
    Ok(Details {
        note_type: crate::type_move::shown_type(&record).0,
        title: record
            .title
            .filter(|t| t.source == TitleSource::User)
            .map(|t| t.text),
        tags: record.tags,
    })
}

/// The notes document as it is now. A run reads it once, when it starts,
/// and sends that copy with every call (R36).
fn session_notes(store: &SharedStore, note_id: &str) -> Result<String, StageError> {
    let notes = lock(store).notes(note_id)?;
    if let Some(result) = crate::recognition::saved_result(store, note_id)?
        && result
            .coverage_limitations
            .iter()
            .any(|note| note == crate::recognition::SCOPED_LABELS_NOTE)
    {
        return Ok(format!(
            "Recognition limitation: {}\n\n{}",
            crate::recognition::SCOPED_LABELS_NOTE,
            notes
        ));
    }
    Ok(notes)
}

/// The title turn's reply as a title: its first line with text, with no
/// heading marks, no `Title:` label, no quotes around it and no full stop
/// at the end, cut to the length that a folder name keeps (KTD7).
pub fn clean_title(reply: &str) -> String {
    const MARKS: &[char] = &['"', '\'', '`', '*', '“', '”', '‘', '’', '«', '»'];
    let line = reply
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    // A bold label, as in `**Title:** Weekly sync`, loses its marks first.
    let mut title = line
        .trim_start_matches('#')
        .trim()
        .trim_start_matches(MARKS);
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

/// Forget a summary, its title proposal and its title request, so the
/// stage runs again from the transcript (R19, R31).
pub fn clear(store: &SharedStore, note_id: &str) -> Result<(), StageError> {
    Ok(lock(store).clear_checkpoints(note_id, Stage::Summarize)?)
}

/// The summarize stage: produce the summary and write it into the working
/// directory, where the save stage picks it up. `queued` is the time that
/// the note waited for its turn in the summary queue, for the request log.
#[allow(clippy::too_many_arguments)]
pub fn run(
    store: &SharedStore,
    note_id: &str,
    session_dir: &Path,
    segments: &[Segment],
    endpoint: SummaryEndpoint,
    active: Prompt,
    control: &dyn Control,
    queued: Duration,
    progress: &mut dyn FnMut(SummaryStep, usize, usize),
) -> Result<(), StageError> {
    let summary = produce(
        store,
        note_id,
        session_dir,
        endpoint,
        active,
        control,
        queued,
        progress,
    )?;
    // `summary.md` has no frontmatter: the save writes the summary data
    // into `longhand.json` (R1).
    let path = session_dir.join(SUMMARY_FILE);
    let temp = session_dir.join(format!("{SUMMARY_FILE}.tmp"));
    std::fs::write(&temp, summary.render_body())?;
    crate::sync_file(&temp)?;
    std::fs::rename(&temp, &path)?;
    checkpoint::put(store, note_id, Stage::Summarize, META, &summary.meta)?;
    checkpoint::mark_complete(store, note_id, Stage::Summarize, segments.len())
}

/// Summarize the `transcript.md` in `folder`, resuming any calls already
/// made. Writes nothing but checkpoints. `active` is the summary prompt of
/// the note type, which the run uses when it saved no snapshot before, or
/// when its inputs changed and every call runs again (KTD3). `queued` is
/// the time that the job waited for its turn in the summary queue.
#[allow(clippy::too_many_arguments)]
pub fn produce(
    store: &SharedStore,
    note_id: &str,
    folder: &Path,
    endpoint: SummaryEndpoint,
    active: Prompt,
    control: &dyn Control,
    queued: Duration,
    progress: &mut dyn FnMut(SummaryStep, usize, usize),
) -> Result<Summary, StageError> {
    crate::follow::follow_folder(&mut lock(store), note_id, false)?;
    let transcript = read_transcript(folder)?;
    let lines = lines_of(&transcript);
    let said = lines.iter().any(|line| matches!(line, Line::Turn(_)));
    let notes = session_notes(store, note_id)?;
    let raw_notes = lock(store).notes(note_id)?;
    checkpoint::put(
        store,
        note_id,
        Stage::Summarize,
        "source-notes",
        &session_core::folder::fingerprint(raw_notes.as_bytes()),
    )?;
    let details = note_details(store, note_id)?;
    // Cached calls stand only for the transcript, the notes and the note
    // details that they were made from. Another computer can replace
    // `transcript.md`, and the notes and the details can change, between a
    // failed run and a regeneration.
    let input = input_fingerprint(&transcript, &notes, &details)?;
    let saved = checkpoint::get::<String>(store, note_id, Stage::Summarize, INPUT)?;
    if saved.as_deref() != Some(input.as_str()) {
        forget_calls(store, note_id)?;
        checkpoint::put(store, note_id, Stage::Summarize, INPUT, &input)?;
        // Every call runs again, so the run uses the prompt that it was
        // given, which is the prompt of the type now (KTD3).
        checkpoint::put(store, note_id, Stage::Summarize, PROMPT, &active)?;
    }
    let client = Client::new(client_endpoint(&endpoint))
        .map_err(|e| StageError::Failed(e.to_string()))?
        .with_retry_waits(endpoint.retry_waits.clone());
    if client.crosses_network_unencrypted() {
        log::warn!(
            "summarizing {note_id} over plain HTTP to {}: the transcript crosses the network unencrypted",
            client.host()
        );
    }
    let runtime = Runtime::new()?;

    let prompt = match checkpoint::get::<Prompt>(store, note_id, Stage::Summarize, PROMPT)? {
        Some(saved) => saved,
        None => {
            // A run from before prompts were saved can have a plan and
            // cached calls made with another prompt. Remove them, so that
            // no result goes out under the name of the active prompt. The
            // stage's completion and failure records stay.
            forget_calls(store, note_id)?;
            checkpoint::put(store, note_id, Stage::Summarize, PROMPT, &active)?;
            active
        }
    };

    // A plan whose calls went to another provider, address or model, or
    // whose budget the context limit setting does not give now, is made
    // again, with its calls (KTD11).
    let source = Source::of(&client);
    let saved = checkpoint::get::<Plan>(store, note_id, Stage::Summarize, PLAN)?;
    let saved = match saved {
        Some(plan) if !plan.agrees_with(endpoint.context_limit, &source) => {
            forget_calls(store, note_id)?;
            None
        }
        other => other,
    };
    let mut plan = match saved {
        Some(plan) => plan,
        None => {
            control.proceed()?;
            let probed = if endpoint.context_limit.is_some() {
                None
            } else {
                // An endpoint that cannot be reached fails the stage before
                // a plan is kept, so the next run probes it again.
                runtime
                    .block_on(
                        context::probe(
                            &client,
                            &Label::note(note_id, RequestKind::Probe).queued(queued),
                        ),
                        control,
                    )?
                    .map_err(|e| StageError::Failed(format!("summarizing: {e}")))?
            };
            let budget = context::resolve(endpoint.context_limit, probed);
            log::info!(
                "summarizing {note_id} with {} on {}: budget {} tokens ({:?})",
                endpoint.model,
                client.host(),
                budget.limit,
                budget.source
            );
            let plan = Plan {
                budget,
                layers: vec![Room::new(budget.limit)],
                source: Some(source),
            };
            checkpoint::put(store, note_id, Stage::Summarize, PLAN, &plan)?;
            plan
        }
    };

    let mut shrinks = 0;
    let mut calls = Calls {
        store,
        note_id,
        client: &client,
        runtime: &runtime,
        control,
        notes: &notes,
        text: &prompt.text,
        title_text: &prompt.title_text,
        background: &prompt.background,
        details: &details,
        ollama: endpoint.provider == Provider::Ollama,
        queued,
        progress: &mut *progress,
        grouped: 0,
    };
    let done = if !said {
        // Nothing was said (AE16): there is nothing to send.
        None
    } else {
        Some(loop {
            match calls.summarize(&lines, &plan) {
                Ok(done) => break done,
                // The model takes less than the budget. Only the user
                // changes the context limit, so the run stops and says so.
                Err(Pressure { layer, room }) => {
                    log::warn!(
                        "{} rejected a layer-{layer} prompt under {} tokens ({:?})",
                        client.host(),
                        room.limit,
                        plan.budget.source
                    );
                    // The default is a guess. A limit that the user set or
                    // that the server reported is too large now.
                    let action = if plan.budget.source == BudgetSource::Default {
                        "enter the model's context length".to_string()
                    } else {
                        format!("enter a limit lower than {}", room.limit)
                    };
                    return Err(StageError::Failed(format!(
                        "the model did not accept {} tokens of text; {action} in \
                         Settings > AI Provider > Context limit",
                        room.limit
                    )));
                }
                Err(CutOff { layer, room }) if shrinks < MAX_SHRINKS => {
                    // The prompt fit and the reply did not: give the reply
                    // more of the same context and send less with it.
                    shrinks += 1;
                    let reserve = (room.reserve * 2).min(room.limit / 2);
                    if reserve <= room.reserve {
                        // The next run asks the endpoint for its limit
                        // again: the user can raise the context length.
                        forget_calls(store, note_id)?;
                        return Err(StageError::Failed(
                            "the model's replies do not fit in its context; raise its context length or choose another model".into(),
                        ));
                    }
                    log::warn!(
                        "{} cut off a layer-{layer} reply at {} tokens; retrying with {reserve}",
                        client.host(),
                        room.reserve
                    );
                    forget_layers_from(store, note_id, layer)?;
                    plan.replace_from(
                        layer,
                        Room {
                            limit: room.limit,
                            reserve,
                        },
                    );
                    checkpoint::put(store, note_id, Stage::Summarize, PLAN, &plan)?;
                }
                Err(CutOff { .. }) => {
                    forget_calls(store, note_id)?;
                    return Err(StageError::Failed(
                        "the endpoint kept cutting off replies".into(),
                    ));
                }
                Err(Stop(e)) => return Err(e),
            }
        })
    };
    // A title problem never fails the summary (R14): an error is logged,
    // and the note keeps its title.
    let text = match done {
        Some(done) => {
            if let Err(e) = calls.store_title_request(&done) {
                log::warn!("could not store the title request of {note_id}: {e}");
                if let Err(e) = delete_title_request(store, note_id) {
                    log::warn!("could not delete the title request of {note_id}: {e}");
                }
            }
            done.reply
        }
        None => {
            // With nothing said, no title turn is due.
            if let Err(e) = delete_title_request(store, note_id) {
                log::warn!("could not delete the title request of {note_id}: {e}");
            }
            String::from("Nothing was transcribed in this note.")
        }
    };

    Ok(Summary {
        meta: SummaryMeta::Complete {
            endpoint_host: client.host(),
            model: endpoint.model.clone(),
            // The name and the id come from the snapshot, so they always
            // match the text that was sent (R11, KTD4). With nothing said,
            // no prompt made the summary.
            prompt_name: said.then(|| prompt.name.clone()),
            prompt_id: said.then(|| prompt.id.clone()),
        },
        body: text,
    })
}

/// Remove the plan, every cached call, the title proposal and the title
/// request.
fn forget_calls(store: &SharedStore, note_id: &str) -> Result<(), StageError> {
    forget_layers_from(store, note_id, 0)?;
    let guard = lock(store);
    guard.delete_checkpoint(note_id, Stage::Summarize, PLAN)?;
    guard.delete_checkpoint(note_id, Stage::Summarize, TITLE)?;
    guard.delete_checkpoint(note_id, Stage::Summarize, TITLE_REQUEST)?;
    Ok(())
}

fn forget_layers_from(store: &SharedStore, note_id: &str, layer: usize) -> Result<(), StageError> {
    let guard = lock(store);
    for (key, _) in guard.checkpoints(note_id, Stage::Summarize)? {
        // The whole call is in layer 0.
        let at = if key == WHOLE {
            Some(0)
        } else {
            key.strip_prefix('l')
                .and_then(|k| k.get(..2))
                .and_then(|l| l.parse::<usize>().ok())
        };
        if at.is_some_and(|l| l >= layer) {
            guard.delete_checkpoint(note_id, Stage::Summarize, &key)?;
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
    note_id: &'a str,
    client: &'a Client,
    runtime: &'a Runtime,
    control: &'a dyn Control,
    /// The notes document as it was when the run started (R36).
    notes: &'a str,
    /// The text of the run's prompt snapshot (KTD4).
    text: &'a str,
    /// The title text of the run's prompt snapshot. Every call that can be
    /// the final call keeps room for the title turn with it (KTD4).
    title_text: &'a str,
    /// The Background of the run's prompt snapshot. Every call carries it,
    /// so every budget counts it (KTD11).
    background: &'a str,
    /// The note details as they were when the run started. Every call
    /// carries them, so every budget counts them.
    details: &'a Details,
    /// The requests go to Ollama, so each request gives its context size
    /// (KTD7).
    ollama: bool,
    /// The time that the job waited for its turn in the summary queue.
    queued: Duration,
    /// Takes the step, the calls of the step done, and the calls of the
    /// step.
    progress: &'a mut dyn FnMut(SummaryStep, usize, usize),
    /// The calls that combined summaries in groups in this attempt, in all
    /// layers before the last.
    grouped: usize,
}

/// The final call: its messages, its reply, which is the summary, and the
/// room that it was built under (KTD3).
struct Final {
    messages: Vec<Message>,
    reply: String,
    room: Room,
}

impl<'a> Calls<'a> {
    /// The user's own texts that every call carries: the Background of the
    /// prompt snapshot and the note details.
    fn user(&self) -> UserText<'a> {
        UserText {
            background: self.background,
            details: self.details,
        }
    }

    /// The Ollama context size for a request that needs `need` tokens under
    /// `room`, or `None` for another provider (KTD7).
    fn num_ctx(&self, need: usize, room: Room) -> Option<u32> {
        self.ollama.then(|| context_size(need, room.limit))
    }

    fn report(&mut self, step: Option<(SummaryStep, usize, usize)>) {
        if let Some((step, done, total)) = step {
            (self.progress)(step, done, total);
        }
    }

    fn summarize(&mut self, lines: &[Line], plan: &Plan) -> Result<Final, Outcome> {
        let (text, title_text) = (self.text, self.title_text);
        let user = self.user();
        let room = plan.room(0);
        // The final call's fixed parts and the prompt text must leave at
        // least half of the prompt allowance for the transcript (KTD8). The
        // title reserve is not part of this check (KTD4).
        if prompts::overhead("", text, user) * 2
            > tokens::prompt_allowance(room.limit, room.reserve)
        {
            // Remove the plan, so that the next run takes the budget from
            // the endpoint again. A model with a larger context can then
            // run the same prompt.
            lock(self.store)
                .delete_checkpoint(self.note_id, Stage::Summarize, PLAN)
                .map_err(StageError::from)?;
            return Err(Stop(StageError::Failed(PROMPT_TOO_LONG.into())));
        }
        let notes = fit_notes(room, self.notes, text, title_text, user);
        let whole = prompts::whole(lines, &notes, text, user);
        let title_reserve = prompts::title_reserve(title_text, user);
        if tokens::with_margin(cost(&whole) + title_reserve) + room.reserve as usize
            <= room.limit as usize
        {
            // One call: its prompt estimate and its reply reserve.
            let num_ctx = self.num_ctx(
                tokens::with_margin(cost(&whole)) + room.reserve as usize,
                room,
            );
            let reply = self.call_as(WHOLE.into(), 0, 0, &whole, room, 1, None, num_ctx)?;
            return Ok(Final {
                messages: whole,
                reply,
                room,
            });
        }

        // Too long for one call: parts, then merges until one remains. Every
        // request of the run gives the room limit as its Ollama context
        // size, so Ollama does not load the model again (KTD7). The cut-off
        // path changes the reply reserve, not the limit.
        let num_ctx = self.num_ctx(room.limit as usize, room);
        let allowance = chunk_allowance(room, &notes, text, title_text, user);
        let chunks: Vec<Chunk> = chunk::chunk(lines, allowance);
        let total = chunks.len();
        let mut summaries = Vec::with_capacity(total);
        (self.progress)(SummaryStep::Parts, 0, total);
        for (i, c) in chunks.iter().enumerate() {
            summaries.push(self.call(
                0,
                i,
                &prompts::part(c, i, total, &notes, user),
                room,
                total,
                Some((SummaryStep::Parts, i + 1, total)),
                num_ctx,
            )?);
        }
        self.grouped = 0;
        let mut layer = 1;
        if summaries.len() == 1 {
            // The margins let one chunk hold what one whole call could not:
            // its notes still need the summary's shape.
            let only = [summaries[0].as_str()];
            let room = plan.room(1);
            let notes = fit_notes(room, self.notes, text, title_text, user);
            let messages = prompts::merge(&only, true, &notes, text, user);
            (self.progress)(SummaryStep::Final, 0, 1);
            let reply = self.call(1, 0, &messages, room, 1, None, num_ctx)?;
            return Ok(Final {
                messages,
                reply,
                room,
            });
        }
        while summaries.len() > 1 {
            let room = plan.room(layer);
            let notes = fit_notes(room, self.notes, text, title_text, user);
            let allowance = chunk_allowance(room, &notes, text, title_text, user);
            let costs: Vec<usize> = summaries.iter().map(|s| tokens::estimate(s) + 8).collect();
            let groups = chunk::merge_groups(&costs, allowance);
            if groups.len() == 1 {
                // The last merge: one group holds every summary.
                let parts: Vec<&str> = summaries.iter().map(String::as_str).collect();
                let messages = prompts::merge(&parts, true, &notes, text, user);
                (self.progress)(SummaryStep::Final, 0, 1);
                let reply = self.call(layer, 0, &messages, room, 1, None, num_ctx)?;
                return Ok(Final {
                    messages,
                    reply,
                    room,
                });
            }
            let mut next = Vec::with_capacity(groups.len());
            // One count for the group calls of all the layers before the
            // last. The calls of later layers are an estimate with replies
            // of the full reserve.
            let steps = self.grouped
                + merge_calls(&costs, allowance, room.reserve as usize + 8).saturating_sub(1);
            (self.progress)(SummaryStep::Groups, self.grouped, steps);
            for (i, g) in groups.iter().enumerate() {
                if g.len() == 1 {
                    next.push(summaries[g.start].clone());
                    continue;
                }
                let parts: Vec<&str> = summaries[g.clone()].iter().map(String::as_str).collect();
                self.grouped += 1;
                next.push(self.call(
                    layer,
                    i,
                    &prompts::merge(&parts, false, &notes, text, user),
                    room,
                    groups.len(),
                    Some((SummaryStep::Groups, self.grouped, steps.max(self.grouped))),
                    num_ctx,
                )?);
            }
            summaries = next;
            layer += 1;
        }
        Err(Stop(StageError::Failed(
            "the transcript gave no summaries to combine".into(),
        )))
    }

    /// Store the title turn of the final call `done` as the pending title
    /// request (KTD3), unless no title turn is due: the user set a title, a
    /// Transcribe Again run keeps the title that the note has (R16, KTD14),
    /// or a title was proposed before. A Transcribe Again of a note with no
    /// title proposes one. The stage sends no call here. When the context
    /// leaves less than [`prompts::TITLE_REPLY_MIN`] for the reply (KTD12),
    /// the stage stores no request and the summary still completes (R14).
    fn store_title_request(&self, done: &Final) -> Result<(), StageError> {
        let proposed =
            checkpoint::get::<String>(self.store, self.note_id, Stage::Summarize, TITLE)?.is_some();
        let (user_title, keeps_title) = {
            let guard = lock(self.store);
            let title = guard.title(self.note_id)?;
            (
                title
                    .as_ref()
                    .is_some_and(|t| t.source == TitleSource::User),
                title.is_some() && guard.transcribe_again_marks(self.note_id)?.model.is_some(),
            )
        };
        if proposed || user_title || keeps_title {
            return delete_title_request(self.store, self.note_id);
        }
        let messages =
            prompts::title_turn(&done.messages, &done.reply, self.title_text, self.user());
        let used = tokens::with_margin(cost(&messages));
        let cap = (done.room.limit as usize)
            .saturating_sub(used)
            .min(done.room.reserve as usize) as u32;
        if cap < prompts::TITLE_REPLY_MIN {
            log::warn!(
                "the title turn for {} leaves {cap} tokens of the context for the reply, under {}; \
                 the session keeps the title it has",
                self.note_id,
                prompts::TITLE_REPLY_MIN
            );
            return delete_title_request(self.store, self.note_id);
        }
        checkpoint::put(
            self.store,
            self.note_id,
            Stage::Summarize,
            TITLE_REQUEST,
            &TitleRequest {
                messages,
                cap,
                limit: Some(done.room.limit),
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn call(
        &mut self,
        layer: usize,
        index: usize,
        messages: &[Message],
        room: Room,
        total: usize,
        step: Option<(SummaryStep, usize, usize)>,
        num_ctx: Option<u32>,
    ) -> Result<String, Outcome> {
        self.call_as(
            call_key(layer, index),
            layer,
            index,
            messages,
            room,
            total,
            step,
            num_ctx,
        )
    }

    /// One call, with its result saved under `key`. When the call is done,
    /// also when its result was saved before, the run reports `step`: the
    /// step, its calls done, and its calls. `num_ctx` is the Ollama context
    /// size of the request (KTD7).
    #[allow(clippy::too_many_arguments)]
    fn call_as(
        &mut self,
        key: String,
        layer: usize,
        index: usize,
        messages: &[Message],
        room: Room,
        total: usize,
        step: Option<(SummaryStep, usize, usize)>,
        num_ctx: Option<u32>,
    ) -> Result<String, Outcome> {
        if let Some(done) =
            checkpoint::get::<String>(self.store, self.note_id, Stage::Summarize, &key)?
        {
            self.report(step);
            return Ok(done);
        }
        self.control.proceed()?;
        let kind = if key == WHOLE {
            RequestKind::Single
        } else if layer == 0 {
            RequestKind::Chunk
        } else {
            RequestKind::Merge
        };
        let label = Label::note(self.note_id, kind).queued(self.queued);
        let reply = self.runtime.block_on(
            self.client
                .complete(&label, messages, room.reserve, num_ctx),
            self.control,
        )?;
        match reply {
            Ok(completion) => {
                checkpoint::put(
                    self.store,
                    self.note_id,
                    Stage::Summarize,
                    &key,
                    &completion.text,
                )?;
                self.report(step);
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
/// lets a prompt use with the prompt `text` and the `user` text (the
/// Background and the note details), after the room for the title turn
/// with `title_text` (KTD4, KTD11).
fn fit_notes(room: Room, notes: &str, text: &str, title_text: &str, user: UserText) -> String {
    prompts::fit_notes(
        notes,
        tokens::prompt_allowance(room.limit, room.reserve)
            .saturating_sub(prompts::title_reserve(title_text, user)),
        text,
        user,
    )
}

/// What is left for the transcript or the summaries in one call, after the
/// prompt's fixed parts, the notes, the prompt `text`, the `user` text (the
/// Background and the note details) and the room for the title turn with
/// `title_text` (KTD4, KTD11).
/// The number of merge calls for summaries that cost `costs` under
/// `allowance`, when each merged summary costs `reply`. The count includes
/// the last merge, and one summary also gets one merge call. A real merged
/// summary is often shorter than `reply`, so the run can need fewer calls.
fn merge_calls(costs: &[usize], allowance: usize, reply: usize) -> usize {
    let mut costs = costs.to_vec();
    let mut calls = 0;
    loop {
        let groups = chunk::merge_groups(&costs, allowance);
        if groups.len() <= 1 {
            return calls + 1;
        }
        calls += groups.iter().filter(|g| g.len() > 1).count();
        costs = groups
            .iter()
            .map(|g| if g.len() == 1 { costs[g.start] } else { reply })
            .collect();
    }
}

fn chunk_allowance(room: Room, notes: &str, text: &str, title_text: &str, user: UserText) -> usize {
    tokens::prompt_allowance(room.limit, room.reserve)
        .saturating_sub(prompts::overhead(notes, text, user))
        .saturating_sub(prompts::title_reserve(title_text, user))
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
    use summarize::chunk::{Line, Turn};

    use super::{Details, clean_title, input_fingerprint, lines_of, merge_calls, read_transcript};

    #[test]
    fn merge_calls_counts_every_layer_and_the_last_merge() {
        // One summary still gets one merge call.
        assert_eq!(merge_calls(&[10], 25, 10), 1);
        // Two groups of two, then one merge of the two results.
        assert_eq!(merge_calls(&[10, 10, 10, 10], 25, 10), 3);
        // A group of one is not a call; it goes on to the next layer.
        assert_eq!(merge_calls(&[10, 10, 10, 10, 10], 25, 10), 4);
        // Everything fits in one group: only the last merge.
        assert_eq!(merge_calls(&[5, 5, 5], 100, 5), 1);
    }

    #[test]
    fn a_saved_plan_is_kept_only_while_the_context_limit_setting_gives_its_budget() {
        use summarize::{Budget, BudgetSource};
        let at = super::Source {
            provider: "custom".into(),
            address: "https://litellm.example/v1".into(),
            model: "m".into(),
        };
        let plan = |limit, source| super::Plan {
            budget: Budget { limit, source },
            layers: vec![super::Room::new(limit)],
            source: Some(at.clone()),
        };
        let default = summarize::context::DEFAULT_LIMIT;
        let cases = [
            // A set limit keeps a plan under the same limit, whatever gave it.
            (plan(32_000, BudgetSource::User), Some(32_000), true),
            (plan(32_000, BudgetSource::Litellm), Some(32_000), true),
            (plan(32_000, BudgetSource::User), Some(64_000), false),
            (plan(default, BudgetSource::Default), Some(32_000), false),
            // With no set limit, a probed plan stays.
            (plan(1_000_000, BudgetSource::Litellm), None, true),
            (plan(default, BudgetSource::Default), None, true),
            // A cleared limit, a learned limit or an older default plans again.
            (plan(32_000, BudgetSource::User), None, false),
            (plan(20_000, BudgetSource::Learned), None, false),
            (plan(8_192, BudgetSource::Default), None, false),
        ];
        for (plan, user, kept) in cases {
            assert_eq!(plan.agrees_with(user, &at), kept, "{plan:?} with {user:?}");
        }
    }

    /// Covers R26, KTD11: a plan is kept only for the provider, the full
    /// address and the model that made its calls. A plan with no source is
    /// from an older version and is made again.
    #[test]
    fn a_saved_plan_is_kept_only_for_the_provider_address_and_model_that_made_it() {
        use summarize::{Budget, BudgetSource};
        let source = |provider: &str, address: &str, model: &str| super::Source {
            provider: provider.into(),
            address: address.into(),
            model: model.into(),
        };
        let at = source("custom", "https://gw.example:8443/v1", "m");
        let plan = |source| super::Plan {
            budget: Budget {
                limit: 32_000,
                source: BudgetSource::User,
            },
            layers: vec![super::Room::new(32_000)],
            source,
        };
        let limit = Some(32_000);
        assert!(plan(Some(at.clone())).agrees_with(limit, &at));
        for other in [
            source("anthropic", "https://gw.example:8443/v1", "m"),
            source("custom", "http://gw.example:8443/v1", "m"),
            source("custom", "https://other.example:8443/v1", "m"),
            source("custom", "https://gw.example:9443/v1", "m"),
            source("custom", "https://gw.example:8443/proxy/v1", "m"),
            source("custom", "https://gw.example:8443/v1", "n"),
        ] {
            assert!(
                !plan(Some(at.clone())).agrees_with(limit, &other),
                "{other:?}"
            );
        }
        assert!(!plan(None).agrees_with(limit, &at));
    }

    /// Covers R6, KTD7: the size is rounded up to a multiple of 2,048, is
    /// at least 4,096, and is never more than the context limit.
    #[test]
    fn the_ollama_context_size_is_rounded_and_kept_inside_the_context_limit() {
        use super::context_size;
        assert_eq!(context_size(19_500, 32_000), 20_480);
        assert_eq!(context_size(20_480, 32_000), 20_480);
        assert_eq!(context_size(20_481, 32_000), 22_528);
        assert_eq!(context_size(900, 32_000), 4_096);
        assert_eq!(context_size(900, 2_048), 2_048);
        assert_eq!(context_size(31_000, 32_000), 32_000);
        assert_eq!(context_size(usize::MAX / 2, 32_000), 32_000);
    }

    /// A note with no details has the fingerprint of an earlier build, so a
    /// checkpoint from that build stays valid.
    #[test]
    fn a_note_with_no_details_has_the_fingerprint_of_an_earlier_build() {
        let earlier = session_core::folder::fingerprint(b"# Transcript\n\0My notes\n");
        assert_eq!(
            input_fingerprint("# Transcript\n", "My notes\n", &Details::default()).unwrap(),
            earlier
        );
        let typed = Details {
            note_type: Some("Pathfinder".into()),
            ..Details::default()
        };
        assert_ne!(
            input_fingerprint("# Transcript\n", "My notes\n", &typed).unwrap(),
            earlier
        );
    }

    #[test]
    fn the_transcript_is_read_from_the_note_files_only() {
        let folder = tempfile::tempdir().unwrap();
        // An older build named result copies that are not in the folder.
        std::fs::write(
            folder.path().join("session.md"),
            "---\nsession_id: s\nstarted: 2026-09-22T14:00:00+00:00\nresult_generation:\n  id: committed-1\n  files:\n    transcript.md: a\n    summary.md: b\nsegments: []\n---\n",
        )
        .unwrap();
        std::fs::write(folder.path().join("transcript.md"), "# Transcript\n").unwrap();
        assert_eq!(read_transcript(folder.path()).unwrap(), "# Transcript\n");
    }

    fn turn(position_ms: u64, speaker: u32, text: &str) -> Line {
        Turn {
            position_ms,
            speaker: Some(speaker),
            text: text.into(),
        }
        .into()
    }

    /// Covers R13: the summary input holds the pause line between the
    /// turns before and after it.
    #[test]
    fn a_pause_line_stays_between_its_turns_in_the_summary_input() {
        let text = "# Transcript\n\n- [00:00:00] Speaker 1: Before.\n\n--- pause 00:14:00 ---\n\n- [00:01:00] Speaker 2: After.\n";
        let lines = lines_of(text);
        assert_eq!(
            lines,
            vec![
                turn(0, 1, "Before."),
                Line::Pause {
                    duration_ms: 840_000
                },
                turn(60_000, 2, "After."),
            ]
        );
        let input = &summarize::prompts::whole(&lines, "", "Summarize.", "")[1].content;
        assert_eq!(
            input.matches("\n--- pause 00:14:00 ---\n").count(),
            1,
            "{input}"
        );
    }

    /// A gap line and a silence line do not go to the summary model.
    #[test]
    fn a_transcript_with_a_gap_line_and_no_pause_gives_only_its_turns() {
        let text = "# Transcript\n\n- [00:00:00] Speaker 1: Before.\n\n--- gap 00:00:30 ---\n\n- [00:01:00] Speaker 2: After.\n\n--- silence 00:00:40 ---\n\n";
        let lines = lines_of(text);
        assert_eq!(
            lines,
            vec![turn(0, 1, "Before."), turn(60_000, 2, "After.")]
        );
    }

    #[test]
    fn a_reply_is_cleaned_to_a_plain_title() {
        assert_eq!(clean_title("  \"Budget Review.\"  "), "Budget Review");
        assert_eq!(clean_title("\"Budget Review\"."), "Budget Review");
        assert_eq!(clean_title("\n# Title: **Q3 plan**\nmore"), "Q3 plan");
        assert_eq!(clean_title("# **Title:** Weekly sync."), "Weekly sync");
        assert_eq!(clean_title("“Hiring  and\tbudget”"), "Hiring and budget");
        assert_eq!(clean_title("Plan v2.0"), "Plan v2.0");
        assert_eq!(clean_title(" \n \"\" "), "");
    }
}
