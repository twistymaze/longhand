//! The passes of the app's summary pipeline, which the lab also runs:
//!
//! - The timeline pipeline: agents write the timeline of each transcript
//!   part, then writers write the summary from the timeline
//!   ([`Pass::timeline`]).
//! - The review pass: auditors check the draft summary against the
//!   transcript, part by part, each in a new context with no tools. Then a
//!   reviser corrects the draft with their findings ([`Pass::review`]).
//!
//! The passes read the transcript from the note in memory, not from a
//! storage folder (KTD2).

use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use summarize::chunk::OVERLAP;
use summarize::client::{Label, Message, RequestKind};
use summarize::prompts::{self, UserText};
use summarize::tokens::{BYTES_PER_TOKEN, ReplyKind};

use crate::shapes::free_tokens;
use crate::{
    Agent, HistoryPolicy, Limits, MemoryNoteSource, Model, Outcome, RunResult, RunTotals, ToolSet,
};

/// The smallest room in tokens for transcript lines in one audit.
const AUDIT_ROOM_MIN: u64 = 1_000;

/// The data of the passes of one note.
pub struct Pass<'a> {
    pub model: &'a dyn Model,
    /// The note: its transcript, manual notes and details. A writer can
    /// read it as a tool source.
    pub note: MemoryNoteSource,
    pub background: String,
    /// The summary prompt's text.
    pub text: String,
    pub limits: Limits,
    /// The label of the turns. Each step sets its own request kind.
    pub label: Label,
    /// The most auditor requests that are open at the same time.
    pub at_once: usize,
    /// The progress hook (R9): the step, the requests done and the
    /// requests of the step. The hook runs on the task of the pass, so it
    /// does not need `Send`; a caller with an `FnMut` puts it in a
    /// `RefCell`.
    pub progress: Option<&'a dyn Fn(Step, usize, usize)>,
}

/// A step of the passes, for the progress hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The timeline parts. The total grows when a part is halved.
    Parts,
    /// The writers of the groups of timeline parts.
    Groups,
    /// The writer or the merge step that writes the summary.
    Final,
    /// The auditors, then the reviser with a total of one.
    Review,
}

/// The result of the timeline pipeline.
pub struct Timeline {
    pub result: RunResult,
    /// The messages of the last writing step (the writer, or the merge
    /// step when groups ran), for the title request. `None` when no
    /// writing step ran.
    pub writer: Option<Vec<Message>>,
}

/// `first`, then `then`: the outcome and the summary of `then`, with the
/// turns and the totals of both.
pub fn join(first: RunResult, then: RunResult) -> RunResult {
    let mut totals = first.totals;
    totals.add_after(&then.totals);
    let mut turns = first.turns;
    turns.extend(then.turns);
    RunResult {
        outcome: then.outcome,
        summary: then.summary,
        detail: then.detail,
        totals,
        turns,
    }
}

impl Pass<'_> {
    fn user(&self) -> UserText<'_> {
        UserText {
            background: &self.background,
            details: &self.note.details,
        }
    }

    fn report(&self, step: Step, done: usize, total: usize) {
        if let Some(hook) = self.progress {
            hook(step, done, total);
        }
    }

    async fn agent(
        &self,
        tools: &ToolSet<'_>,
        mut messages: Vec<Message>,
        name: &str,
        kind: RequestKind,
    ) -> RunResult {
        Agent {
            model: self.model,
            tools,
            preamble: std::mem::take(&mut messages[0].content),
            prompt: std::mem::take(&mut messages[1].content),
            limits: self.limits,
            history: HistoryPolicy::default(),
            label: Label {
                kind,
                ..self.label.clone()
            },
            name: name.into(),
        }
        .run()
        .await
    }

    /// A writer or a merge step, which can call `tools` to check one point.
    /// A reply with no tool call is a summary. When the model cannot use
    /// the tools, or a tool result fills the context, or the reply is cut
    /// off, or the run reaches a call limit, the step runs again with no
    /// tools: its first request fits by design, so the step can still end
    /// with a summary.
    async fn writer(&self, tools: &ToolSet<'_>, messages: Vec<Message>, name: &str) -> RunResult {
        let mut result = self
            .agent(tools, messages.clone(), name, RequestKind::Writer)
            .await;
        if matches!(
            result.outcome,
            Outcome::NoToolSupport
                | Outcome::MalformedCalls
                | Outcome::ContextExceeded
                | Outcome::CutOff
                | Outcome::ToolCallLimit
        ) {
            let no_tools = ToolSet::empty();
            let again = self
                .agent(&no_tools, messages, name, RequestKind::Writer)
                .await;
            result = join(result, again);
        }
        if result.outcome == Outcome::NoReads {
            result.outcome = Outcome::Summary;
        }
        result
    }

    /// The note's share of a writer request: what the Background, the note
    /// details and the manual notes add to a request with no timeline. The
    /// free context already keeps room for the fixed rules.
    fn writer_note_share(&self) -> u64 {
        let id = &self.note.note_id;
        let request = |notes: &str, user: UserText<'_>| {
            prompts::cost(&prompts::from_timeline(
                "", notes, id, None, &self.text, user,
            )) as u64
        };
        request(&self.note.manual_notes, self.user()).saturating_sub(request("", "".into()))
    }

    /// The lines of one timeline part: the free context less the note's
    /// share of a timeline part request and the people list, and at most
    /// four times the output maximum, because a timeline is about a quarter
    /// of its lines. The note's share is what the Background, the note
    /// details and the manual notes add to a request with no lines; the
    /// free context already keeps room for the fixed rules. The lines get
    /// at least a quarter of the free context, also when the note's share
    /// is near the budget (KTD8).
    fn part_lines(&self, lines: usize, chars: usize) -> usize {
        let request = |notes: &str, user: UserText<'_>| {
            prompts::cost(&prompts::timeline_part(
                lines, lines, lines, "", notes, "", user,
            )) as u64
        };
        let note_share =
            request(&self.note.manual_notes, self.user()).saturating_sub(request("", "".into()));
        let free = free_tokens(&self.limits);
        let mut tokens = free
            .saturating_sub(note_share + PEOPLE_TOKENS)
            .max(free / PART_FLOOR_SHARE);
        if let Some(output) = self.limits.output_max {
            tokens = tokens.min(u64::from(output) * 4);
        }
        // Quoting makes a line about a tenth longer.
        let line_bytes = chars.max(1) as f64 / lines.max(1) as f64 * 1.1;
        ((tokens as f64 * BYTES_PER_TOKEN / line_bytes) as usize).max(1)
    }

    /// The timeline pipeline for a transcript of `lines` lines and `chars`
    /// characters. The code runs each step and keeps each output in memory:
    ///
    /// 1. Timeline parts, in order: each part of the transcript that fits
    ///    in one context goes to a new agent with no tools, with the manual
    ///    notes and the people list of the part before. It writes the people
    ///    list so far and the timeline of its lines. A part whose reply is
    ///    cut off runs again with half the lines, at most [`MAX_SHRINKS`]
    ///    times in a run. With a local reply kind, the request already had
    ///    all the room of the context, so a cut-off ends the run at once
    ///    (KTD9).
    /// 2. When the joined timeline fits in a writer's context, one writer
    ///    writes the summary from it. Otherwise one writer for each group of
    ///    parts writes notes, and a merge step writes the summary.
    ///
    /// The writers and the merge step can call `tools` to check a point. A
    /// failed step ends the run with its outcome.
    pub async fn timeline(&self, tools: &ToolSet<'_>, lines: usize, chars: usize) -> Timeline {
        let ended = |result| Timeline {
            result,
            writer: None,
        };
        let no_tools = ToolSet::empty();
        let local = matches!(self.limits.reply_kind, ReplyKind::Full | ReplyKind::Ollama);
        let mut done = RunResult::empty();
        let mut size = self.part_lines(lines, chars);
        let mut shrinks = 0;
        let mut total = parts_from(1, lines, size);
        self.report(Step::Parts, 0, total);
        let mut parts = Vec::new();
        let mut people = String::new();
        let mut first = 1;
        while first <= lines {
            let last = (first + size - 1).min(lines);
            let text = match self.note.lines(first, last) {
                Ok(text) => text,
                Err(error) => return ended(failed(done, error)),
            };
            let messages = prompts::timeline_part(
                first,
                last,
                lines,
                &text,
                &self.note.manual_notes,
                &people,
                self.user(),
            );
            let name = format!("timeline {}", parts.len() + 1);
            let result = self
                .agent(&no_tools, messages, &name, RequestKind::Timeline)
                .await;
            let cut_off = result.outcome == Outcome::CutOff;
            let part = result.written();
            done = join(done, result);
            match part {
                Some(part) => {
                    let (list, timeline) = split_people(&part);
                    if let Some(list) = list {
                        people = list;
                    }
                    parts.push(timeline);
                    self.report(Step::Parts, parts.len(), total);
                }
                None if cut_off && last > first && !local && shrinks < MAX_SHRINKS => {
                    shrinks += 1;
                    size = (last - first).div_ceil(2);
                    total = parts.len() + parts_from(first, lines, size);
                    continue;
                }
                None => {
                    done.summary = None;
                    return ended(done);
                }
            }
            if last == lines {
                break;
            }
            first = next_first(first, last);
        }
        let with_people = |timeline: &str| {
            if people.is_empty() {
                timeline.to_string()
            } else {
                format!("People:\n{people}\n\n{timeline}")
            }
        };
        let room = free_tokens(&self.limits).saturating_sub(self.writer_note_share());
        let tokens = |text: &str| (text.len() as f64 / BYTES_PER_TOKEN) as u64;
        let whole = with_people(&parts.join("\n"));
        if tokens(&whole) <= room * 2 / 3 {
            let messages = prompts::from_timeline(
                &whole,
                &self.note.manual_notes,
                &self.note.note_id,
                None,
                &self.text,
                self.user(),
            );
            self.report(Step::Final, 0, 1);
            let written = self.writer(tools, messages.clone(), "writer").await;
            return Timeline {
                result: join(done, written),
                writer: Some(messages),
            };
        }
        let groups = groups(&parts, room / 2, tokens);
        let count = groups.len();
        // All group notes together take at most half the merge step's room;
        // about 0.75 words for each token.
        let share = room / 2 / count.max(1) as u64;
        let words = (share * 3 / 4).max(1) as usize;
        let most = ((share as f64 * BYTES_PER_TOKEN) as usize).max(1);
        let mut group_notes = Vec::new();
        self.report(Step::Groups, 0, count);
        for (index, group) in groups.iter().enumerate() {
            let messages = prompts::from_timeline(
                &with_people(group),
                &self.note.manual_notes,
                &self.note.note_id,
                Some((index, count, words)),
                prompts::GROUP_TEXT,
                self.user(),
            );
            let result = self
                .writer(tools, messages, &format!("writer {}", index + 1))
                .await;
            let notes = result.written();
            done = join(done, result);
            match notes {
                // Notes longer than their share of the merge request are
                // cut, so that the merge request fits.
                Some(notes) if notes.chars().count() > most => {
                    let kept: String = notes.chars().take(most).collect();
                    group_notes.push(format!("{kept}\n[These notes were cut here.]"));
                }
                Some(notes) => group_notes.push(notes),
                None => {
                    done.summary = None;
                    return ended(done);
                }
            }
            self.report(Step::Groups, index + 1, count);
        }
        let notes: Vec<&str> = group_notes.iter().map(String::as_str).collect();
        let messages = prompts::step_merge(&notes, &self.note.note_id, &self.text, self.user());
        self.report(Step::Final, 0, 1);
        let merged = self.writer(tools, messages.clone(), "merge").await;
        Timeline {
            result: join(done, merged),
            writer: Some(messages),
        }
    }

    /// Check `draft` against the transcript of `lines` lines and `chars`
    /// characters, and correct it. The auditors are independent: up to
    /// [`Pass::at_once`] of them run at the same time, and their findings
    /// join in part order (KTD11). The result has the corrected summary,
    /// or the draft when the auditors find no error. A failed auditor or
    /// reviser keeps the draft, and the detail says why.
    pub async fn review(&self, draft: &str, lines: usize, chars: usize) -> RunResult {
        let no_tools = ToolSet::empty();
        let fixed = (draft.len() + self.note.manual_notes.len()) as f64 / BYTES_PER_TOKEN;
        let room = free_tokens(&self.limits).saturating_sub(fixed as u64);
        let mut done = RunResult::empty();
        if room < AUDIT_ROOM_MIN || lines == 0 {
            return self.kept(draft, done, "no room in the context for an audit".into());
        }
        let line_bytes = chars.max(1) as f64 / lines as f64;
        let part = ((room as f64 * BYTES_PER_TOKEN / line_bytes) as usize).max(1);
        let ranges: Vec<(usize, usize)> = (1..=lines)
            .step_by(part)
            .map(|first| (first, (first + part - 1).min(lines)))
            .collect();
        let total = ranges.len();
        self.report(Step::Review, 0, total);
        let no_tools = &no_tools;
        let mut found: Vec<Option<String>> = vec![None; total];
        let mut finished = 0;
        let mut open = FuturesUnordered::new();
        let mut next = ranges.iter().enumerate();
        loop {
            while open.len() < self.at_once.max(1) {
                let Some((slot, &(first, last))) = next.next() else {
                    break;
                };
                let text = match self.note.lines(first, last) {
                    Ok(text) => text,
                    Err(error) => return self.kept(draft, done, error),
                };
                let messages = prompts::audit(
                    first,
                    last,
                    lines,
                    &text,
                    draft,
                    &self.note.manual_notes,
                    self.user(),
                );
                open.push(async move {
                    let name = format!("auditor {}", slot + 1);
                    let result = self
                        .agent(no_tools, messages, &name, RequestKind::Audit)
                        .await;
                    (slot, result)
                });
            }
            let Some((slot, result)) = open.next().await else {
                break;
            };
            let reply = result.written();
            let failed = format!("{:?}", result.outcome);
            done = join(done, result);
            match reply {
                Some(text) if !text.trim().trim_matches('.').eq_ignore_ascii_case("none") => {
                    found[slot] = Some(text.trim().to_string());
                }
                Some(_) => {}
                None => {
                    let detail = format!("auditor {} stopped: {failed}", slot + 1);
                    return self.kept(draft, done, detail);
                }
            }
            finished += 1;
            self.report(Step::Review, finished, total);
        }
        let findings: Vec<String> = found.into_iter().flatten().collect();
        if findings.is_empty() {
            done.outcome = Outcome::Summary;
            done.summary = Some(draft.to_string());
            done.detail = None;
            return done;
        }
        let messages = prompts::revise(
            draft,
            &findings.join("\n"),
            &self.note.manual_notes,
            self.user(),
        );
        self.report(Step::Review, 0, 1);
        let result = self
            .agent(no_tools, messages, "reviser", RequestKind::Revise)
            .await;
        let reply = result.written();
        let failed = format!("the reviser stopped: {:?}", result.outcome);
        let mut done = join(done, result);
        let Some(reply) = reply else {
            return self.kept(draft, done, failed);
        };
        match apply_edits(draft, &reply) {
            Ok((revised, applied, total)) => {
                done.outcome = Outcome::Summary;
                done.summary = Some(revised);
                done.detail = Some(format!("review: {applied} of {total} edits applied"));
                done
            }
            Err(error) => self.kept(draft, done, error),
        }
    }

    /// `done` with the draft as the summary and `detail` as the reason.
    fn kept(&self, draft: &str, mut done: RunResult, detail: String) -> RunResult {
        done.outcome = Outcome::Summary;
        done.summary = Some(draft.to_string());
        done.detail = Some(detail);
        done
    }
}

/// How many times a run halves a timeline part whose reply is cut off, as
/// in the app's summary step.
pub const MAX_SHRINKS: usize = 6;

/// The lines of a timeline part get at least this share of the free
/// context: one quarter (KTD8).
const PART_FLOOR_SHARE: u64 = 4;

/// The first line of the part after the part `first`..=`last`. The next
/// part repeats about a tenth of this part, as the app's chunks do, so
/// that no exchange is cut in two. The parts get no extra instruction for
/// it; the writer merges the repeats.
fn next_first(first: usize, last: usize) -> usize {
    let repeat = ((last - first + 1) as f64 * OVERLAP).round() as usize;
    (last + 1 - repeat).max(first + 1)
}

/// The number of timeline parts of `size` lines from the line `first` to
/// the line `lines`.
fn parts_from(mut first: usize, lines: usize, size: usize) -> usize {
    let mut count = 0;
    while first <= lines {
        count += 1;
        let last = (first + size - 1).min(lines);
        if last == lines {
            break;
        }
        first = next_first(first, last);
    }
    count
}

/// The tokens kept free for the people list in a timeline part.
const PEOPLE_TOKENS: u64 = 1_000;

/// The `<people>` block of a timeline part and the timeline without it.
/// A reply with no block has no people list.
fn split_people(reply: &str) -> (Option<String>, String) {
    let (Some(start), Some(end)) = (reply.find("<people>"), reply.find("</people>")) else {
        return (None, reply.trim().to_string());
    };
    if end < start {
        return (None, reply.trim().to_string());
    }
    let people = reply[start + "<people>".len()..end].trim().to_string();
    let timeline = format!("{}{}", &reply[..start], &reply[end + "</people>".len()..]);
    (Some(people), timeline.trim().to_string())
}

/// `draft` with the edits of a reviser `reply`: a JSON object with a list
/// of `find` and `replace` texts. An edit applies only when its `find`
/// text is in the draft exactly once. The result has the new text, the
/// number of edits applied and the number of edits.
pub fn apply_edits(draft: &str, reply: &str) -> Result<(String, usize, usize), String> {
    let start = reply.find('{').ok_or("the reviser gave no JSON object")?;
    let end = reply.rfind('}').ok_or("the reviser gave no JSON object")?;
    if end < start {
        return Err("the reviser gave no JSON object".into());
    }
    let value: serde_json::Value = serde_json::from_str(&reply[start..=end])
        .map_err(|e| format!("the reviser's reply is not valid JSON: {e}"))?;
    let edits = value["edits"]
        .as_array()
        .ok_or("the reviser's reply has no edits list")?;
    let mut text = draft.to_string();
    let mut applied = 0;
    for edit in edits {
        let (Some(find), Some(replace)) = (edit["find"].as_str(), edit["replace"].as_str()) else {
            continue;
        };
        if find.trim().is_empty() || text.matches(find).count() != 1 {
            continue;
        }
        text = text.replacen(find, replace, 1);
        applied += 1;
    }
    Ok((text, applied, edits.len()))
}

/// The timeline `parts` in consecutive groups of at most `most` tokens
/// each. A part larger than `most` is a group by itself.
fn groups(parts: &[String], most: u64, tokens: impl Fn(&str) -> u64) -> Vec<String> {
    let mut groups: Vec<String> = Vec::new();
    let mut current = String::new();
    for part in parts {
        if !current.is_empty() && tokens(&current) + tokens(part) > most {
            groups.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(part);
    }
    if !current.is_empty() {
        groups.push(current);
    }
    groups
}

/// `done` as a run that ended with no summary, for the reason `detail`.
fn failed(mut done: RunResult, detail: String) -> RunResult {
    done.outcome = Outcome::NoSummary;
    done.summary = None;
    done.detail = Some(detail);
    done
}

impl RunResult {
    /// The summary when the outcome has one, else `None`.
    pub fn written(&self) -> Option<String> {
        self.summary.clone().filter(|_| self.outcome.has_summary())
    }

    /// A result with no turns, to add other results to.
    pub fn empty() -> Self {
        RunResult {
            outcome: Outcome::Summary,
            summary: None,
            detail: None,
            totals: RunTotals::default(),
            turns: Vec::new(),
        }
    }
}
