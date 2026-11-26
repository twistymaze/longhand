//! The summarize stage (KTD16, R17, R18, R19, R24b). The stage reads the
//! transcript from `transcript.md` (KTD4), finds the context budget, and
//! runs the timeline pipeline of `summary_agent` (KTD1 of the summary
//! pipeline plan): timeline parts in order, a writer (or group writers and
//! a merge step), then the review. The pipeline reads the transcript that
//! the stage read, not a storage folder (KTD2), so a first summary runs in
//! the working folder before Save.
//!
//! The turn layer ([`turns`]) saves the reply of each model turn, so a run
//! that stops continues from the last completed turn (R6). A rejection that
//! shows that the model's context is smaller than the budget stops the run:
//! only the user changes the context limit.
//!
//! The notes document goes with each request (KTD6), as it was when the
//! run started (R36), cut to fit the budget (KTD8). The note details (the
//! shown type, the tags and a title that the user typed) go with each
//! request in the same way. After the summary, the stage stores the title
//! turn as a pending title request, unless the user set a title, and sends
//! no title call (KTD3). The title job sends the request after the save.
//!
//! A run takes a snapshot of the active summary prompt before its first
//! request and keeps it to the end, also after a resume (KTD4). The
//! snapshot's text goes to the writer and the merge step, and `summary.md`
//! records its name.

use std::cell::RefCell;
use std::path::Path;
use std::time::Duration;

use library::scan::{READ_TIMEOUT, Read, read_timed};
use serde::{Deserialize, Serialize};
use session_core::folder::{Summary, SummaryMeta, Transcript, TranscriptLine};
use session_core::{Segment, SessionRecord, Stage, TitleSource};
use summarize::chunk::{Line, Turn};
use summarize::client::{Label, Message, RequestKind, Thinking};
use summarize::prompts::{Details, UserText};
use summarize::tokens::ReplyKind;
use summarize::{
    Budget, BudgetSource, Client, Endpoint, OpenRouterRouting, Provider, SummarizeError,
    THINKING_SETTING, context, prompts, tokens,
};
use summary_agent::passes::{Pass, Step};
use summary_agent::{Limits, MemoryNoteSource, Outcome, ToolSet, ToolSource};

use crate::join::TRANSCRIPT_FILE;
use crate::{Control, SharedStore, StageError, checkpoint, lock};

pub mod turns;

pub use summarize::Prompt;

pub const SUMMARY_FILE: &str = "summary.md";
/// The context budget of the run, a `SavedBudget` (KTD5): the budget, the
/// source of the requests, the Ollama context size and the output maximum.
/// A resumed run uses it, so its requests are the same as before.
pub const BUDGET: &str = "budget";
/// The snapshot of the summary prompt that the run uses, a [`Prompt`]
/// (KTD4). It is saved before the first request, and a resumed run uses it
/// in place of the active prompt.
pub const PROMPT: &str = "prompt";
/// The fingerprint of the raw manual notes that the run read, a string.
pub const SOURCE_NOTES: &str = "source-notes";
/// The keys of the part, merge and final method of older builds (KTD7):
/// the plan and the whole-transcript call. Their part and merge calls have
/// the keys of [`is_layer_key`]. This build deletes them.
const OLD_KEYS: [&str; 2] = ["plan", "whole"];
/// The method of the summary, in the value under [`INPUT`] (KTD7). An
/// older build compares the value with its own fingerprint, sees a changed
/// input, and forgets its calls.
const METHOD: &str = "timeline-pipeline-1";
/// The error when the fixed parts of the writer's request and the prompt
/// text leave too little of the model's context for the transcript (KTD8,
/// KTD10). The prompt text and the Background both count (KTD11).
const PROMPT_TOO_LONG: &str = "the summary prompt is too long for this model's context; \
shorten the prompt or the Background in Settings, or choose a model with a larger context";
/// The error when the model ends the run with no summary and the user can
/// do nothing about it (KTD10). The log holds the cause.
const NO_SUMMARY: &str = "the model did not write a summary";
/// The error when a reply is cut off at the largest allowance that the
/// context and the provider permit (R4, KTD8).
fn reply_does_not_fit() -> String {
    format!(
        "the model's reply did not fit in its context; \
         give the model a larger context, or choose a lower Thinking level in {THINKING_SETTING}"
    )
}
/// The error when a summary request reaches its reply deadline (R16,
/// KTD10). The request is not sent again.
fn no_reply_in_time() -> String {
    format!("the model did not reply in time; choose a lower Thinking level in {THINKING_SETTING}")
}
/// The proposed title, a string, once the title turn has run. An empty
/// string means that the reply held no title.
pub const TITLE: &str = "title";
/// The pending title turn, a [`TitleRequest`] (KTD3). The Summarize stage
/// stores it after the summary, and the title job sends it and deletes it.
/// It is a row in the store, so it stays after Save removes the working
/// folder.
pub const TITLE_REQUEST: &str = session_core::jobs::TITLE_REQUEST;
/// The method and the fingerprint of the input to the requests, a string:
/// the transcript, the notes and the note details. A run whose input has
/// another value makes every request again.
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
const CANCEL_POLL: Duration = Duration::from_millis(250);

/// `client` with the OpenRouter routing rules of `endpoint`, when it has
/// any.
fn with_routing(client: Client, endpoint: &SummaryEndpoint) -> Client {
    match endpoint.openrouter {
        Some(routing) => client.with_openrouter(routing),
        None => client,
    }
}

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
    /// The Thinking level of the summary requests and the title request,
    /// and whether the user chose it (R5, R6, R8, KTD7, KTD11).
    pub thinking: Thinking,
    /// How many requests go to the provider at the same time (R12).
    pub requests_at_once: u32,
    /// The waits before each new try of a request that could not reach the
    /// endpoint ([`summarize::client::RETRY_WAITS`] in the app).
    pub retry_waits: Vec<Duration>,
    /// The OpenRouter routing rules of the requests. The app sets none; a
    /// dev tool sets them to keep note text away from hosts that store
    /// request data.
    pub openrouter: Option<OpenRouterRouting>,
}

/// Where the requests of a run went: the provider, the full address
/// (scheme, host, port and path) and the model (KTD11).
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
/// prompt estimate and the reply allowance (KTD7). The size is a multiple of
/// [`CONTEXT_STEP`], at least [`SMALLEST_CONTEXT`], and never more than
/// `limit`, the context limit.
fn context_size(need: usize, limit: u32) -> u32 {
    let rounded = u32::try_from(need.div_ceil(CONTEXT_STEP) * CONTEXT_STEP).unwrap_or(u32::MAX);
    rounded.max(SMALLEST_CONTEXT).min(limit)
}

/// The context budget that a run committed to (KTD5), so a resumed run
/// sends the same requests and reuses their saved replies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SavedBudget {
    budget: Budget,
    /// The provider, the address and the model of the requests (KTD11).
    source: Source,
    /// The Ollama context size of each request of the run, or `None` for
    /// another provider (KTD7, KTD9). One value for the run, so Ollama
    /// does not load the model again.
    num_ctx: Option<u32>,
    /// The smaller of the probe's and the client's output maximum.
    output_max: Option<u32>,
}

impl SavedBudget {
    /// True when the requests went to `source` and the context limit
    /// setting `user` still gives this budget. A set limit keeps a budget
    /// under the same limit, whatever gave it. With no set limit, a budget
    /// under a set limit or under an older default is made again.
    fn agrees_with(&self, user: Option<u32>, source: &Source) -> bool {
        if &self.source != source {
            return false;
        }
        match (user, self.budget.source) {
            (Some(limit), _) => self.budget.limit == limit,
            (None, BudgetSource::User | BudgetSource::Learned) => false,
            (None, BudgetSource::Default) => self.budget.limit == context::DEFAULT_LIMIT,
            (None, _) => true,
        }
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
    /// The context limit of the run, which limits the Ollama context size
    /// of the title turn (KTD7). A request from an older version has none.
    pub limit: Option<u32>,
    /// The Ollama context size of the run's requests. The title turn uses
    /// it, so Ollama does not load the model again (KTD12). A request from
    /// an older version, or for another provider, has none. An older build
    /// ignores the field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_ctx: Option<u32>,
}

/// The stored form of a [`TitleRequest`]. A [`Message`] has a fixed role
/// text, so the read maps each role to it.
#[derive(Deserialize)]
struct StoredTitleRequest {
    messages: Vec<StoredMessage>,
    cap: u32,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    num_ctx: Option<u32>,
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
        num_ctx: stored.num_ctx,
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
        Ok(client) => with_routing(client.with_thinking(endpoint.thinking), endpoint),
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
    // The title turn uses the context size of the run. A request from an
    // older version needs its prompt and its reply cap. With no saved
    // limit, the limit of the setting applies, or none.
    let num_ctx = (endpoint.provider == Provider::Ollama).then(|| {
        request.num_ctx.unwrap_or_else(|| {
            context_size(
                tokens::with_margin(prompts::cost(&request.messages)) + request.cap as usize,
                request.limit.or(endpoint.context_limit).unwrap_or(u32::MAX),
            )
        })
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

/// The step of the summary pipeline that a progress report is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryStep {
    /// One request for each timeline part of the transcript. The total
    /// grows when a part is halved.
    Parts,
    /// The writers of the groups of timeline parts, when the whole
    /// timeline does not fit in one writer's context.
    Groups,
    /// The writer or the merge step, which writes the summary.
    Final,
    /// The auditors, which run at the same time. `done` is the number of
    /// audit parts that are complete. The reviser reports a total of one.
    Review,
}

impl SummaryStep {
    pub fn name(self) -> &'static str {
        match self {
            Self::Parts => "parts",
            Self::Groups => "groups",
            Self::Final => "final",
            Self::Review => "review",
        }
    }
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
pub fn lines_of(text: &str) -> Vec<Line> {
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
pub fn note_details(store: &SharedStore, note_id: &str) -> Result<Details, StageError> {
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

/// Summarize the `transcript.md` in `folder` with the timeline pipeline,
/// and reuse the saved reply of each turn that a run made before. Writes
/// nothing but checkpoints. `active` is the summary prompt of the note
/// type, which the run uses when it saved no snapshot before, or when its
/// inputs changed and every request runs again (KTD3). `queued` is the
/// time that the job waited for its turn in the summary queue.
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
    let said = lines_of(&transcript)
        .iter()
        .any(|line| matches!(line, Line::Turn(_)));
    let notes = session_notes(store, note_id)?;
    let raw_notes = lock(store).notes(note_id)?;
    checkpoint::put(
        store,
        note_id,
        Stage::Summarize,
        SOURCE_NOTES,
        &session_core::folder::fingerprint(raw_notes.as_bytes()),
    )?;
    let details = note_details(store, note_id)?;
    // Saved turns stand only for the transcript, the notes and the note
    // details that they were made from. Another computer can replace
    // `transcript.md`, and the notes and the details can change, between a
    // failed run and a regeneration.
    let input = input_value(&transcript, &notes, &details)?;
    let saved = checkpoint::get::<String>(store, note_id, Stage::Summarize, INPUT)?;
    if saved.as_deref() != Some(input.as_str()) {
        forget_calls(store, note_id)?;
        checkpoint::put(store, note_id, Stage::Summarize, INPUT, &input)?;
        // Every request runs again, so the run uses the prompt that it was
        // given, which is the prompt of the type now (KTD3).
        checkpoint::put(store, note_id, Stage::Summarize, PROMPT, &active)?;
    }
    // A run of an older build can leave its plan and its calls (R15).
    forget_old_method(store, note_id)?;
    let client = Client::new(client_endpoint(&endpoint))
        .map_err(|e| StageError::Failed(e.to_string()))?
        .with_retry_waits(endpoint.retry_waits.clone())
        .with_thinking(endpoint.thinking);
    let client = with_routing(client, &endpoint);
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
            // A run from before prompts were saved can have saved replies
            // made with another prompt. Remove them, so that no result goes
            // out under the name of the active prompt. The stage's
            // completion and failure records stay.
            forget_calls(store, note_id)?;
            checkpoint::put(store, note_id, Stage::Summarize, PROMPT, &active)?;
            active
        }
    };
    let meta = |body: String| Summary {
        meta: SummaryMeta::Complete {
            endpoint_host: client.host(),
            model: endpoint.model.clone(),
            // The name and the id come from the snapshot, so they always
            // match the text that was sent (R11, KTD4). With nothing said,
            // no prompt made the summary.
            prompt_name: said.then(|| prompt.name.clone()),
            prompt_id: said.then(|| prompt.id.clone()),
        },
        body,
    };
    if !said {
        // Nothing was said (AE16, R17): there is nothing to send, and no
        // title turn is due.
        if let Err(e) = delete_title_request(store, note_id) {
            log::warn!("could not delete the title request of {note_id}: {e}");
        }
        return Ok(meta(String::from("Nothing was transcribed in this note.")));
    }

    let budget = run_budget(
        store, note_id, &endpoint, &client, &runtime, control, queued,
    )?;
    let limit = budget.budget.limit;
    let user = UserText {
        background: &prompt.background,
        details: &details,
    };
    // The fixed parts of the writer's request and the prompt text must
    // leave at least half of the prompt allowance for the timeline (KTD8,
    // KTD10). The title reserve is not part of this check (KTD4).
    let allowance = tokens::prompt_allowance(limit, tokens::split_reserve(limit));
    if prompts::overhead("", &prompt.text, user) * 2 > allowance {
        // The next run takes the budget from the endpoint again. A model
        // with a larger context can then run the same prompt.
        lock(store).delete_checkpoint(note_id, Stage::Summarize, BUDGET)?;
        return Err(StageError::Failed(PROMPT_TOO_LONG.into()));
    }
    let notes = prompts::fit_notes(
        &notes,
        allowance.saturating_sub(prompts::title_reserve(&prompt.title_text, user)),
        &prompt.text,
        user,
    );
    let started = {
        let record = note_record(store, note_id)?;
        let segments = lock(store).segments(note_id)?;
        record.start(&segments).to_rfc3339()
    };
    let reply_kind = ReplyKind::of(endpoint.provider, budget.budget.source);
    let turns = turns::Turns::new(store, note_id, &client, control);
    let steps = RefCell::new(progress);
    let report = |step: Step, done: usize, total: usize| {
        let step = match step {
            Step::Parts => SummaryStep::Parts,
            Step::Groups => SummaryStep::Groups,
            Step::Final => SummaryStep::Final,
            Step::Review => SummaryStep::Review,
        };
        (*steps.borrow_mut())(step, done, total);
    };
    let pass = Pass {
        model: &turns,
        note: MemoryNoteSource::new(
            note_id.to_string(),
            started,
            transcript,
            notes,
            details.clone(),
            tokens::result_chars(limit),
        ),
        background: prompt.background.clone(),
        text: prompt.text.clone(),
        limits: Limits {
            context_tokens: u64::from(limit),
            output_max: budget.output_max,
            num_ctx: budget.num_ctx,
            reply_kind,
            ..Limits::default()
        },
        label: Label::note(note_id, RequestKind::AgentTurn).queued(queued),
        at_once: endpoint.requests_at_once.max(1) as usize,
        progress: Some(&report),
    };
    let tools = ToolSet::new(vec![&pass.note as &dyn ToolSource]).map_err(StageError::Failed)?;
    let (lines, chars) = (pass.note.transcript_lines(), pass.note.transcript_chars());

    turns.clear_error();
    let timeline = runtime.block_on(pass.timeline(&tools, lines, chars), control)?;
    // The agent gets a cancel as an error, so a cancel is checked before
    // the outcome is used (R7).
    if turns.cancelled() || control.is_cancelled() {
        return Err(StageError::Cancelled);
    }
    let result = timeline.result;
    let Some(draft) = result.written() else {
        log::warn!(
            "the summary of {note_id} stopped: {:?} ({})",
            result.outcome,
            result.detail.as_deref().unwrap_or("no detail")
        );
        let error = turns.error();
        if matches!(error, Some(SummarizeError::ReplyCutOff)) {
            // The next run asks the endpoint for its limit again: the user
            // can give the model a larger context.
            lock(store).delete_checkpoint(note_id, Stage::Summarize, BUDGET)?;
        }
        return Err(StageError::Failed(failure(
            result.outcome,
            error,
            &budget.budget,
        )));
    };

    turns.clear_error();
    let review = runtime.block_on(pass.review(&draft, lines, chars), control)?;
    // A cancel during the review never publishes the draft (R7).
    if turns.cancelled() || control.is_cancelled() {
        return Err(StageError::Cancelled);
    }
    if let Some(detail) = &review.detail {
        log::info!("the review of the summary of {note_id}: {detail}");
    }
    let body = review.written().unwrap_or(draft);

    // A title problem never fails the summary (R14): an error is logged,
    // and the note keeps its title.
    let title = TitleTurn {
        writer: timeline.writer.as_deref(),
        summary: &body,
        title_text: &prompt.title_text,
        user,
        budget: &budget,
        reply_kind,
    };
    if let Err(e) = store_title_request(store, note_id, &title) {
        log::warn!("could not store the title request of {note_id}: {e}");
        if let Err(e) = delete_title_request(store, note_id) {
            log::warn!("could not delete the title request of {note_id}: {e}");
        }
    }
    Ok(meta(body))
}

/// The saved budget of the run, or a new one (KTD5, KTD6). A saved budget
/// whose requests went to another provider, address or model, or that
/// the context limit setting does not give now, is made again, and the
/// saved replies go with it (KTD11). A new budget is the user's context
/// limit, else the catalog limit of an OpenAI or Anthropic model, else the
/// probe, else the default. An endpoint that cannot be reached fails the
/// stage before a budget is saved, so the next run probes it again.
#[allow(clippy::too_many_arguments)]
fn run_budget(
    store: &SharedStore,
    note_id: &str,
    endpoint: &SummaryEndpoint,
    client: &Client,
    runtime: &Runtime,
    control: &dyn Control,
    queued: Duration,
) -> Result<SavedBudget, StageError> {
    let source = Source::of(client);
    let saved = checkpoint::get::<SavedBudget>(store, note_id, Stage::Summarize, BUDGET)?;
    match saved {
        Some(saved) if saved.agrees_with(endpoint.context_limit, &source) => return Ok(saved),
        Some(_) => forget_calls(store, note_id)?,
        None => {}
    }
    control.proceed()?;
    let found = if endpoint.context_limit.is_some() {
        None
    } else if let Some(budget) = context::catalog_budget(endpoint.provider, &endpoint.model) {
        Some(budget)
    } else {
        runtime
            .block_on(
                context::probe(
                    client,
                    &Label::note(note_id, RequestKind::Probe).queued(queued),
                ),
                control,
            )?
            .map_err(|e| StageError::Failed(format!("summarizing: {e}")))?
    };
    let budget = context::resolve(endpoint.context_limit, found);
    log::info!(
        "summarizing {note_id} with {} on {}: budget {} tokens ({:?})",
        endpoint.model,
        client.host(),
        budget.limit,
        budget.source
    );
    let saved = SavedBudget {
        budget,
        source,
        num_ctx: (endpoint.provider == Provider::Ollama)
            .then(|| context_size(budget.limit as usize, budget.limit)),
        output_max: [budget.output_max, client.output_max()]
            .into_iter()
            .flatten()
            .min(),
    };
    checkpoint::put(store, note_id, Stage::Summarize, BUDGET, &saved)?;
    Ok(saved)
}

/// The message of a run that ended with no summary (KTD10). The typed
/// error of the last turn decides it, when that turn failed. Outcomes with
/// no action for the user give one message.
fn failure(outcome: Outcome, error: Option<SummarizeError>, budget: &Budget) -> String {
    match error {
        Some(e) if e.is_context_pressure() => context_limit_message(budget),
        Some(SummarizeError::ReplyCutOff) => reply_does_not_fit(),
        Some(SummarizeError::Deadline(_)) => no_reply_in_time(),
        Some(e) => format!("summarizing: {e}"),
        // The agent found that its next request does not fit the budget.
        None if outcome == Outcome::ContextExceeded => context_limit_message(budget),
        None => NO_SUMMARY.into(),
    }
}

/// The message when the model takes less than the budget. Only the user
/// changes the context limit, so the run stops and says so.
fn context_limit_message(budget: &Budget) -> String {
    // The default is a guess. A limit that the user set, that the catalog
    // gave or that the server reported is too large now.
    let action = if budget.source == BudgetSource::Default {
        "enter the model's context length".to_string()
    } else {
        format!("enter a limit lower than {}", budget.limit)
    };
    format!(
        "the model did not accept {} tokens of text; {action} in \
         Settings > AI Provider > Context limit",
        budget.limit
    )
}

/// True for the key of a part or merge call of an older build: `l`, two
/// digits, `-` and five digits (KTD7). A turn key never has this form.
fn is_layer_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    bytes.len() == 9
        && bytes[0] == b'l'
        && bytes[1..3].iter().all(u8::is_ascii_digit)
        && bytes[3] == b'-'
        && bytes[4..].iter().all(u8::is_ascii_digit)
}

/// True for a key of the part, merge and final method of older builds.
fn is_old_method_key(key: &str) -> bool {
    OLD_KEYS.contains(&key) || is_layer_key(key)
}

/// Remove the keys of the part, merge and final method of older builds
/// (R15, KTD7).
fn forget_old_method(store: &SharedStore, note_id: &str) -> Result<(), StageError> {
    let guard = lock(store);
    for (key, _) in guard.checkpoints(note_id, Stage::Summarize)? {
        if is_old_method_key(&key) {
            guard.delete_checkpoint(note_id, Stage::Summarize, &key)?;
        }
    }
    Ok(())
}

/// Remove the budget, every saved turn, the keys of the older method, the
/// title proposal and the title request.
fn forget_calls(store: &SharedStore, note_id: &str) -> Result<(), StageError> {
    let guard = lock(store);
    for (key, _) in guard.checkpoints(note_id, Stage::Summarize)? {
        if key.starts_with(turns::TURN_PREFIX) || is_old_method_key(&key) {
            guard.delete_checkpoint(note_id, Stage::Summarize, &key)?;
        }
    }
    for key in [BUDGET, TITLE, TITLE_REQUEST] {
        guard.delete_checkpoint(note_id, Stage::Summarize, key)?;
    }
    Ok(())
}

/// What the title turn is made from (KTD12).
struct TitleTurn<'a> {
    /// The messages of the last writing step: the writer, or the merge
    /// step. `None` when no writing step ran.
    writer: Option<&'a [Message]>,
    /// The summary after the review.
    summary: &'a str,
    title_text: &'a str,
    user: UserText<'a>,
    budget: &'a SavedBudget,
    reply_kind: ReplyKind,
}

/// Store the title turn as the pending title request (KTD3, KTD12), unless
/// no title turn is due: the user set a title, a Transcribe Again run
/// keeps the title that the note has (R16, KTD14), or a title was proposed
/// before. A Transcribe Again of a note with no title proposes one. The
/// stage sends no call here. When the context of the run leaves less than
/// [`prompts::TITLE_REPLY_MIN`] for the reply, the stage stores no request
/// and the summary still completes (R14).
fn store_title_request(
    store: &SharedStore,
    note_id: &str,
    turn: &TitleTurn,
) -> Result<(), StageError> {
    let proposed = checkpoint::get::<String>(store, note_id, Stage::Summarize, TITLE)?.is_some();
    let (user_title, keeps_title) = {
        let guard = lock(store);
        let title = guard.title(note_id)?;
        (
            title
                .as_ref()
                .is_some_and(|t| t.source == TitleSource::User),
            title.is_some() && guard.transcribe_again_marks(note_id)?.model.is_some(),
        )
    };
    let Some(writer) = turn.writer else {
        return delete_title_request(store, note_id);
    };
    if proposed || user_title || keeps_title {
        return delete_title_request(store, note_id);
    }
    let messages = prompts::title_turn(writer, turn.summary, turn.title_text, turn.user);
    // The title request gets the first allowance of the same rule as a
    // summary request, under the run's budget (KTD11, KTD12).
    let limit = turn.budget.budget.limit;
    let cap = tokens::reply_allowance(
        limit,
        tokens::with_margin(prompts::cost(&messages)),
        turn.budget.output_max,
        turn.reply_kind,
    )
    .first;
    if cap < prompts::TITLE_REPLY_MIN {
        log::warn!(
            "the title turn for {note_id} leaves {cap} tokens of the context for the reply, under {}; \
             the note keeps the title it has",
            prompts::TITLE_REPLY_MIN
        );
        return delete_title_request(store, note_id);
    }
    checkpoint::put(
        store,
        note_id,
        Stage::Summarize,
        TITLE_REQUEST,
        &TitleRequest {
            messages,
            cap,
            limit: Some(limit),
            num_ctx: turn.budget.num_ctx,
        },
    )
}

/// The value under [`INPUT`]: the method tag and the fingerprint of the
/// input (KTD7). An older build stores the fingerprint alone.
fn input_value(transcript: &str, notes: &str, details: &Details) -> Result<String, StageError> {
    Ok(format!(
        "{METHOD}:{}",
        input_fingerprint(transcript, notes, details)?
    ))
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
    use summarize::{Budget, BudgetSource, SummarizeError};
    use summary_agent::Outcome;

    use super::{
        Details, SavedBudget, Source, clean_title, failure, input_fingerprint, input_value,
        is_old_method_key, lines_of, read_transcript,
    };

    fn budget(limit: u32, source: BudgetSource) -> Budget {
        Budget {
            limit,
            source,
            output_max: None,
        }
    }

    /// Covers KTD10, R16: a deadline failure names the Thinking setting.
    /// That a deadline sends no second request is shown in the client
    /// tests (`an_endpoint_that_never_answers_hits_the_deadline`).
    #[test]
    fn a_deadline_failure_names_the_thinking_setting() {
        for what in ["response", "data", "complete reply"] {
            let text = failure(
                Outcome::Transport,
                Some(SummarizeError::Deadline(what)),
                &budget(32_000, BudgetSource::User),
            );
            assert!(text.contains("Thinking"), "{text}");
            assert!(text.contains(summarize::THINKING_SETTING), "{text}");
        }
    }

    /// Covers R10, KTD10: the typed error of the last failed turn gives
    /// the message. With no failed turn, a context check of the agent gives
    /// the Context limit message, and the other outcomes give one message.
    #[test]
    fn the_typed_error_of_the_last_turn_gives_the_failure_message() {
        let set = budget(32_000, BudgetSource::User);
        let default = budget(summarize::context::DEFAULT_LIMIT, BudgetSource::Default);
        let pressure = failure(
            Outcome::ContextExceeded,
            Some(SummarizeError::ContextExceeded),
            &set,
        );
        assert!(
            pressure.contains("enter a limit lower than 32000")
                && pressure.contains("Settings > AI Provider > Context limit"),
            "{pressure}"
        );
        let guess = failure(Outcome::ContextExceeded, None, &default);
        assert!(
            guess.contains("enter the model's context length"),
            "{guess}"
        );
        let cut = failure(Outcome::CutOff, Some(SummarizeError::ReplyCutOff), &set);
        assert_eq!(cut, super::reply_does_not_fit());
        let offline = failure(
            Outcome::Transport,
            Some(SummarizeError::Transport("connection refused".into())),
            &set,
        );
        assert!(
            offline.contains("could not reach the endpoint"),
            "{offline}"
        );
        for outcome in [
            Outcome::ToolCallLimit,
            Outcome::MalformedCalls,
            Outcome::NoSummary,
            Outcome::CutOff,
        ] {
            assert_eq!(
                failure(outcome, None, &set),
                "the model did not write a summary"
            );
        }
    }

    fn saved(limit: u32, source: BudgetSource, at: &Source) -> SavedBudget {
        SavedBudget {
            budget: budget(limit, source),
            source: at.clone(),
            num_ctx: None,
            output_max: None,
        }
    }

    /// Covers KTD5: a saved budget is kept only while the context limit
    /// setting gives it.
    #[test]
    fn a_saved_budget_is_kept_only_while_the_context_limit_setting_gives_it() {
        let at = Source {
            provider: "custom".into(),
            address: "https://litellm.example/v1".into(),
            model: "m".into(),
        };
        let default = summarize::context::DEFAULT_LIMIT;
        let cases = [
            // A set limit keeps a budget under the same limit, whatever
            // gave it.
            (saved(32_000, BudgetSource::User, &at), Some(32_000), true),
            (
                saved(32_000, BudgetSource::Litellm, &at),
                Some(32_000),
                true,
            ),
            (saved(32_000, BudgetSource::User, &at), Some(64_000), false),
            (
                saved(default, BudgetSource::Default, &at),
                Some(32_000),
                false,
            ),
            // With no set limit, a probed or catalog budget stays.
            (saved(1_000_000, BudgetSource::Litellm, &at), None, true),
            (saved(400_000, BudgetSource::Catalog, &at), None, true),
            (saved(default, BudgetSource::Default, &at), None, true),
            // A cleared limit or an older default makes a new budget.
            (saved(32_000, BudgetSource::User, &at), None, false),
            (saved(8_192, BudgetSource::Default, &at), None, false),
        ];
        for (budget, user, kept) in cases {
            assert_eq!(
                budget.agrees_with(user, &at),
                kept,
                "{budget:?} with {user:?}"
            );
        }
    }

    /// Covers R26, KTD11: a budget is kept only for the provider, the full
    /// address and the model of its requests.
    #[test]
    fn a_saved_budget_is_kept_only_for_the_provider_address_and_model_of_its_requests() {
        let source = |provider: &str, address: &str, model: &str| Source {
            provider: provider.into(),
            address: address.into(),
            model: model.into(),
        };
        let at = source("custom", "https://gw.example:8443/v1", "m");
        let limit = Some(32_000);
        assert!(saved(32_000, BudgetSource::User, &at).agrees_with(limit, &at));
        for other in [
            source("anthropic", "https://gw.example:8443/v1", "m"),
            source("custom", "http://gw.example:8443/v1", "m"),
            source("custom", "https://other.example:8443/v1", "m"),
            source("custom", "https://gw.example:9443/v1", "m"),
            source("custom", "https://gw.example:8443/proxy/v1", "m"),
            source("custom", "https://gw.example:8443/v1", "n"),
        ] {
            assert!(
                !saved(32_000, BudgetSource::User, &at).agrees_with(limit, &other),
                "{other:?}"
            );
        }
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

    /// Covers R14, KTD7: the value under `input` has a method tag, so an
    /// older build, which stores the fingerprint alone, sees a changed
    /// input and forgets its calls. The details still change the value.
    #[test]
    fn the_input_value_differs_from_the_value_of_an_older_build() {
        let earlier = session_core::folder::fingerprint(b"# Transcript\n\0My notes\n");
        assert_eq!(
            input_fingerprint("# Transcript\n", "My notes\n", &Details::default()).unwrap(),
            earlier
        );
        let value = input_value("# Transcript\n", "My notes\n", &Details::default()).unwrap();
        assert_ne!(value, earlier);
        assert!(value.ends_with(&earlier), "{value}");
        let typed = Details {
            note_type: Some("Pathfinder".into()),
            ..Details::default()
        };
        assert_ne!(
            input_value("# Transcript\n", "My notes\n", &typed).unwrap(),
            value
        );
    }

    /// Covers R15, KTD7: the keys of the older method are a named list and
    /// the exact layer key form. No turn key and no other key of the stage
    /// matches.
    #[test]
    fn only_the_keys_of_the_older_method_are_old_method_keys() {
        for key in ["plan", "whole", "l00-00000", "l12-00345"] {
            assert!(is_old_method_key(key), "{key}");
        }
        for key in [
            "turn-0123456789abcdef",
            "turn-l00-00000",
            "l00-0000",
            "l0a-00000",
            "l00-000001",
            "budget",
            "prompt",
            "input",
            super::SOURCE_NOTES,
            "title",
            "title_request",
            "meta",
            "done",
        ] {
            assert!(!is_old_method_key(key), "{key}");
        }
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

    /// Covers R13: the lines of a transcript hold the pause line between
    /// the turns before and after it.
    #[test]
    fn a_pause_line_stays_between_its_turns_in_the_lines() {
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
