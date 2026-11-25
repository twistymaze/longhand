//! The passes around an agent run that the lab tests:
//!
//! - The timeline pipeline: agents write the timeline of each transcript
//!   part, then writers write the summary from the timeline
//!   ([`Pass::timeline`]).
//! - The review pass: auditors check the draft summary against the
//!   transcript, part by part, each in a new context with no tools. Then a
//!   reviser corrects the draft with their findings ([`Pass::review`]).

use std::path::PathBuf;

use summarize::chunk::OVERLAP;
use summarize::client::{Label, Message};
use summarize::prompts::{self, Details, UserText};
use summarize::tokens::BYTES_PER_TOKEN;

use crate::shapes::{free_tokens, transcript_lines};
use crate::{Agent, HistoryPolicy, Limits, Model, Outcome, RunResult, RunTotals, ToolSet};

/// The smallest room in tokens for transcript lines in one audit.
const AUDIT_ROOM_MIN: u64 = 1_000;

/// The data of the passes of one note.
pub struct Pass<'a> {
    pub model: &'a dyn Model,
    pub root: PathBuf,
    pub note_id: String,
    pub background: String,
    pub details: Details,
    pub manual_notes: String,
    /// The summary prompt's text.
    pub text: String,
    pub limits: Limits,
    pub label: Label,
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
            details: &self.details,
        }
    }

    async fn agent(&self, tools: &ToolSet<'_>, messages: Vec<Message>, name: &str) -> RunResult {
        Agent {
            model: self.model,
            tools,
            preamble: messages[0].content.clone(),
            prompt: messages[1].content.clone(),
            limits: self.limits,
            history: HistoryPolicy::default(),
            label: self.label.clone(),
            name: name.into(),
        }
        .run()
        .await
    }

    /// A writer or a merge step, which can call `tools` to check one point.
    /// A reply with no tool call is a summary. When the model cannot use
    /// the tools, or a tool result fills the context, or the reply is cut
    /// off, the step runs again with no tools: its first request fits by
    /// design, so the step can still end with a summary.
    async fn writer(&self, tools: &ToolSet<'_>, messages: Vec<Message>, name: &str) -> RunResult {
        let mut result = self.agent(tools, messages.clone(), name).await;
        if matches!(
            result.outcome,
            Outcome::NoToolSupport
                | Outcome::MalformedCalls
                | Outcome::ContextExceeded
                | Outcome::CutOff
        ) && let Ok(no_tools) = ToolSet::new(Vec::new())
        {
            let again = self.agent(&no_tools, messages, name).await;
            result = join(result, again);
        }
        if result.outcome == Outcome::NoReads {
            result.outcome = Outcome::Summary;
        }
        result
    }

    /// The lines of one timeline part: the free context less the manual
    /// notes and the people list, and at most four times the output
    /// maximum, because a timeline is about a quarter of its lines.
    fn part_lines(&self, lines: usize, chars: usize) -> usize {
        let notes = (self.manual_notes.len() as f64 / BYTES_PER_TOKEN) as u64;
        let mut tokens = free_tokens(&self.limits).saturating_sub(notes + PEOPLE_TOKENS);
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
    ///    cut off runs again with half the lines.
    /// 2. When the joined timeline fits in a writer's context, one writer
    ///    writes the summary from it. Otherwise one writer for each group of
    ///    parts writes notes, and a merge step writes the summary.
    ///
    /// The writers and the merge step can call `tools` to check a point. A
    /// failed step ends the run with its outcome.
    pub async fn timeline(&self, tools: &ToolSet<'_>, lines: usize, chars: usize) -> RunResult {
        let no_tools = match ToolSet::new(Vec::new()) {
            Ok(tools) => tools,
            Err(error) => return failed(RunResult::empty(), error),
        };
        let mut done = RunResult::empty();
        let mut size = self.part_lines(lines, chars);
        let mut parts = Vec::new();
        let mut people = String::new();
        let mut first = 1;
        while first <= lines {
            let last = (first + size - 1).min(lines);
            let text = match transcript_lines(&self.root, &self.note_id, first, last) {
                Ok(text) => text,
                Err(error) => return failed(done, error),
            };
            let messages = prompts::timeline_part(
                first,
                last,
                lines,
                &text,
                &self.manual_notes,
                &people,
                self.user(),
            );
            let name = format!("timeline {}", parts.len() + 1);
            let result = self.agent(&no_tools, messages, &name).await;
            let cut_off = result.outcome == Outcome::CutOff;
            let part = result
                .summary
                .clone()
                .filter(|_| result.outcome.has_summary());
            done = join(done, result);
            match part {
                Some(part) => {
                    let (list, timeline) = split_people(&part);
                    if let Some(list) = list {
                        people = list;
                    }
                    parts.push(timeline);
                }
                None if cut_off && last > first => {
                    size = (last - first).div_ceil(2);
                    continue;
                }
                None => {
                    done.summary = None;
                    return done;
                }
            }
            if last == lines {
                break;
            }
            // The next part repeats about a tenth of this part, as the app's
            // chunks do, so that no exchange is cut in two. The parts get no
            // extra instruction for it; the writer merges the repeats.
            let repeat = ((last - first + 1) as f64 * OVERLAP).round() as usize;
            first = (last + 1 - repeat).max(first + 1);
        }
        let with_people = |timeline: &str| {
            if people.is_empty() {
                timeline.to_string()
            } else {
                format!("People:\n{people}\n\n{timeline}")
            }
        };
        let notes = self.manual_notes.len() as f64 / BYTES_PER_TOKEN;
        let room = free_tokens(&self.limits).saturating_sub(notes as u64);
        let tokens = |text: &str| (text.len() as f64 / BYTES_PER_TOKEN) as u64;
        let whole = with_people(&parts.join("\n"));
        if tokens(&whole) <= room * 2 / 3 {
            let messages = prompts::from_timeline(
                &whole,
                &self.manual_notes,
                &self.note_id,
                None,
                &self.text,
                self.user(),
            );
            return join(done, self.writer(tools, messages, "writer").await);
        }
        let groups = groups(&parts, room / 2, tokens);
        let count = groups.len();
        // All group notes together take at most half the merge step's room;
        // about 0.75 words for each token.
        let share = room / 2 / count.max(1) as u64;
        let words = (share * 3 / 4).max(1) as usize;
        let most = ((share as f64 * BYTES_PER_TOKEN) as usize).max(1);
        let mut group_notes = Vec::new();
        for (index, group) in groups.iter().enumerate() {
            let messages = prompts::from_timeline(
                &with_people(group),
                &self.manual_notes,
                &self.note_id,
                Some((index, count, words)),
                prompts::GROUP_TEXT,
                self.user(),
            );
            let result = self
                .writer(tools, messages, &format!("writer {}", index + 1))
                .await;
            let notes = result
                .summary
                .clone()
                .filter(|_| result.outcome.has_summary());
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
                    return done;
                }
            }
        }
        let notes: Vec<&str> = group_notes.iter().map(String::as_str).collect();
        let messages = prompts::step_merge(&notes, &self.note_id, &self.text, self.user());
        join(done, self.writer(tools, messages, "merge").await)
    }

    /// Check `draft` against the transcript of `lines` lines and `chars`
    /// characters, and correct it. The result has the corrected summary, or
    /// the draft when the auditors find no error. A failed auditor or
    /// reviser keeps the draft, and the detail says why.
    pub async fn review(&self, draft: &str, lines: usize, chars: usize) -> RunResult {
        let no_tools = match ToolSet::new(Vec::new()) {
            Ok(tools) => tools,
            Err(error) => return self.kept(draft, RunResult::empty(), error),
        };
        let fixed = (draft.len() + self.manual_notes.len()) as f64 / BYTES_PER_TOKEN;
        let room = free_tokens(&self.limits).saturating_sub(fixed as u64);
        let mut done = RunResult::empty();
        if room < AUDIT_ROOM_MIN || lines == 0 {
            return self.kept(draft, done, "no room in the context for an audit".into());
        }
        let line_bytes = chars.max(1) as f64 / lines as f64;
        let part = ((room as f64 * BYTES_PER_TOKEN / line_bytes) as usize).max(1);
        let mut findings = Vec::new();
        let mut first = 1;
        let mut number = 0;
        while first <= lines {
            let last = (first + part - 1).min(lines);
            number += 1;
            let text = match transcript_lines(&self.root, &self.note_id, first, last) {
                Ok(text) => text,
                Err(error) => return self.kept(draft, done, error),
            };
            let messages = prompts::audit(
                first,
                last,
                lines,
                &text,
                draft,
                &self.manual_notes,
                self.user(),
            );
            let result = self
                .agent(&no_tools, messages, &format!("auditor {number}"))
                .await;
            let found = result
                .summary
                .clone()
                .filter(|_| result.outcome.has_summary());
            let failed = format!("{:?}", result.outcome);
            done = join(done, result);
            match found {
                Some(text) if !text.trim().trim_matches('.').eq_ignore_ascii_case("none") => {
                    findings.push(text.trim().to_string());
                }
                Some(_) => {}
                None => {
                    return self.kept(draft, done, format!("auditor {number} stopped: {failed}"));
                }
            }
            first = last + 1;
        }
        if findings.is_empty() {
            done.outcome = Outcome::Summary;
            done.summary = Some(draft.to_string());
            done.detail = None;
            return done;
        }
        let messages =
            prompts::revise(draft, &findings.join("\n"), &self.manual_notes, self.user());
        let result = self.agent(&no_tools, messages, "reviser").await;
        let reply = result
            .summary
            .clone()
            .filter(|_| result.outcome.has_summary());
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
