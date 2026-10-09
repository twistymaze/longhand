//! Packing whole turns under a budget.

use summarize::chunk::{Chunk, Line, Turn, chunk, render_line};
use summarize::client::Message;
use summarize::prompts;
use summarize::tokens::{estimate, prompt_allowance, reply_reserve};

fn turns(n: usize) -> Vec<Line> {
    (0..n)
        .map(|i| {
            Turn {
                position_ms: i as u64 * 360,
                speaker: Some((i % 5) as u32 + 1),
                text: format!("turn {i} says something about the plan for the next session"),
            }
            .into()
        })
        .collect()
}

fn cost(chunk: &Chunk) -> usize {
    chunk
        .lines
        .iter()
        .map(|l| estimate(&render_line(l)) + 1)
        .sum::<usize>()
}

#[test]
fn a_four_hour_transcript_packs_under_a_small_budget_without_splitting_a_turn() {
    let all = turns(40_000);
    let allowance = prompt_allowance(4_096, reply_reserve(4_096)) - 400;
    let chunks = chunk(&all, allowance);
    assert!(chunks.len() > 100);
    assert!(chunks.iter().all(|c| cost(c) <= allowance));
    // The owned parts, in order, are exactly the transcript.
    let owned: Vec<&Line> = chunks
        .iter()
        .flat_map(|c| c.lines[c.first_new..].iter())
        .collect();
    assert_eq!(owned.len(), all.len());
    assert!(owned.iter().zip(&all).all(|(a, b)| *a == b));
    // Neighbours share about a tenth of their turns.
    for pair in chunks.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        let shared = b.first_new;
        let expected = (a.lines.len() as f64 * 0.1).round() as usize;
        assert!(shared.abs_diff(expected) <= 1, "{shared} vs {expected}");
        assert_eq!(&b.lines[..shared], &a.lines[a.lines.len() - shared..]);
    }
}

#[test]
fn one_turn_longer_than_a_chunk_splits_at_sentences_keeping_label_and_time() {
    let sentence = "This is one sentence of a very long monologue that goes on. ";
    let long = Turn {
        position_ms: 7_200_000,
        speaker: Some(3),
        text: sentence.repeat(200).trim().to_string(),
    };
    let chunks = chunk(&[long.clone().into()], 300);
    assert!(chunks.len() > 1);
    let parts: Vec<&Turn> = chunks
        .iter()
        .flat_map(|c| c.lines[c.first_new..].iter())
        .map(|l| match l {
            Line::Turn(t) => t,
            Line::Pause { .. } => panic!("no pause in the input"),
        })
        .collect();
    assert!(
        parts
            .iter()
            .all(|p| p.speaker == Some(3) && p.position_ms == 7_200_000)
    );
    assert!(parts.iter().all(|p| p.text.ends_with("goes on.")));
    assert!(chunks.iter().all(|c| cost(c) <= 300));
    let rejoined: Vec<&str> = parts.iter().map(|p| p.text.as_str()).collect();
    assert_eq!(rejoined.join(" "), long.text);
}

/// The turns of `turns(n)` with a 14-minute pause line before every
/// seventh turn.
fn with_pauses(n: usize) -> Vec<Line> {
    let mut out = Vec::new();
    for (i, line) in turns(n).into_iter().enumerate() {
        if i % 7 == 6 {
            out.push(Line::Pause {
                duration_ms: 840_000,
            });
        }
        out.push(line);
    }
    out
}

#[test]
fn chunking_never_puts_a_pause_line_at_the_end_of_a_part() {
    let all = with_pauses(2_000);
    // Many allowances, so that many chunk ends meet a pause line.
    for allowance in (200..1_400).step_by(37) {
        let chunks = chunk(&all, allowance);
        assert!(chunks.len() > 10);
        for c in &chunks {
            assert!(
                matches!(c.lines.last(), Some(Line::Turn(_))),
                "{allowance}: {:?}",
                c.lines.last()
            );
            assert!(cost(c) <= allowance);
        }
        let owned: Vec<&Line> = chunks
            .iter()
            .flat_map(|c| c.lines[c.first_new..].iter())
            .collect();
        assert_eq!(owned.len(), all.len());
        assert!(owned.iter().zip(&all).all(|(a, b)| *a == b));
    }
}

/// A part with a pause line carries the pause rule. The chunk costs leave
/// room for it, so each part request still fits.
#[test]
fn each_part_request_with_pause_lines_fits_the_allowance() {
    let all = with_pauses(2_000);
    let allowance = prompt_allowance(4_096, reply_reserve(4_096));
    let chunks = chunk(&all, allowance - prompts::overhead(NOTES, &topic(), ""));
    assert!(chunks.len() > 2);
    for (i, c) in chunks.iter().enumerate() {
        let messages = prompts::part(c, i, chunks.len(), NOTES, "");
        assert!(messages[1].content.contains("not recorded"));
        assert!(request_cost(&messages) <= allowance);
    }
}

fn request_cost(messages: &[Message]) -> usize {
    summarize::tokens::estimate_messages(messages.iter().map(|m| m.content.as_str()))
}

const NOTES: &str = "# Plan\n\n- the decision\n- and why\n";

#[test]
fn each_chunk_request_contains_the_notes_document_once() {
    let all = turns(2_000);
    let allowance = prompt_allowance(4_096, reply_reserve(4_096));
    let notes = prompts::fit_notes(NOTES, allowance, &topic(), "");
    assert_eq!(notes, NOTES);
    let chunks = chunk(&all, allowance - prompts::overhead(&notes, &topic(), ""));
    assert!(chunks.len() > 2);
    for (i, c) in chunks.iter().enumerate() {
        let messages = prompts::part(c, i, chunks.len(), &notes, "");
        let user = &messages[1].content;
        assert_eq!(user.matches(NOTES).count(), 1, "{user}");
        assert_eq!(user.matches("<notes>").count(), 1);
        assert!(request_cost(&messages) <= allowance);
    }
}

#[test]
fn a_notes_document_longer_than_the_allowance_is_cut_with_a_marker_and_the_requests_fit() {
    let all = turns(2_000);
    let allowance = prompt_allowance(4_096, reply_reserve(4_096));
    let long: String = (0..2_000)
        .map(|i| format!("- note {i} about something the user found important\n"))
        .collect();
    assert!(estimate(&long) > allowance);
    let notes = prompts::fit_notes(&long, allowance, &topic(), "");
    assert!(notes.len() < long.len());
    assert!(
        notes.ends_with(prompts::NOTES_CUT),
        "{}",
        &notes[notes.len() - 80..]
    );
    // The cut keeps whole lines of the document.
    let kept = notes.strip_suffix(prompts::NOTES_CUT).unwrap();
    assert!(long.starts_with(kept) && kept.ends_with('\n'));
    // Room remains for the transcript.
    let room = allowance - prompts::overhead(&notes, &topic(), "");
    assert!(room >= allowance / 3, "{room} of {allowance}");
    let chunks = chunk(&all, room);
    for (i, c) in chunks.iter().enumerate() {
        let messages = prompts::part(c, i, chunks.len(), &notes, "");
        assert!(messages[1].content.contains(prompts::NOTES_CUT));
        assert!(request_cost(&messages) <= allowance);
    }
    assert!(request_cost(&prompts::merge(&[], true, &notes, &topic(), "")) <= allowance);
}

/// The default prompt text: the final call reserves room for it.
fn topic() -> String {
    prompts::built_in(prompts::TOPIC_SUMMARY_ID).unwrap().text
}
