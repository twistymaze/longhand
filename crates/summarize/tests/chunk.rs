//! Packing whole turns under a budget.

use summarize::chunk::{Chunk, Turn, chunk, render_turn};
use summarize::client::Message;
use summarize::prompts;
use summarize::tokens::{estimate, prompt_allowance, reply_reserve};

fn turns(n: usize) -> Vec<Turn> {
    (0..n)
        .map(|i| Turn {
            position_ms: i as u64 * 360,
            speaker: Some((i % 5) as u32 + 1),
            text: format!("turn {i} says something about the plan for the next session"),
        })
        .collect()
}

fn cost(chunk: &Chunk) -> usize {
    chunk
        .turns
        .iter()
        .map(|t| estimate(&render_turn(t)) + 1)
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
    let owned: Vec<&Turn> = chunks
        .iter()
        .flat_map(|c| c.turns[c.first_new..].iter())
        .collect();
    assert_eq!(owned.len(), all.len());
    assert!(owned.iter().zip(&all).all(|(a, b)| *a == b));
    // Neighbours share about a tenth of their turns.
    for pair in chunks.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        let shared = b.first_new;
        let expected = (a.turns.len() as f64 * 0.1).round() as usize;
        assert!(shared.abs_diff(expected) <= 1, "{shared} vs {expected}");
        assert_eq!(&b.turns[..shared], &a.turns[a.turns.len() - shared..]);
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
    let chunks = chunk(std::slice::from_ref(&long), 300);
    assert!(chunks.len() > 1);
    let parts: Vec<&Turn> = chunks
        .iter()
        .flat_map(|c| c.turns[c.first_new..].iter())
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

fn request_cost(messages: &[Message]) -> usize {
    summarize::tokens::estimate_messages(messages.iter().map(|m| m.content.as_str()))
}

const NOTES: &str = "# Plan\n\n- the decision\n- and why\n";

#[test]
fn each_chunk_request_contains_the_notes_document_once() {
    let all = turns(2_000);
    let allowance = prompt_allowance(4_096, reply_reserve(4_096));
    let notes = prompts::fit_notes(NOTES, allowance);
    assert_eq!(notes, NOTES);
    let chunks = chunk(&all, allowance - prompts::overhead(&notes));
    assert!(chunks.len() > 2);
    for (i, c) in chunks.iter().enumerate() {
        let messages = prompts::part(c, i, chunks.len(), &notes);
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
    let notes = prompts::fit_notes(&long, allowance);
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
    let room = allowance - prompts::overhead(&notes);
    assert!(room >= allowance / 3, "{room} of {allowance}");
    let chunks = chunk(&all, room);
    for (i, c) in chunks.iter().enumerate() {
        let messages = prompts::part(c, i, chunks.len(), &notes);
        assert!(messages[1].content.contains(prompts::NOTES_CUT));
        assert!(request_cost(&messages) <= allowance);
    }
    assert!(request_cost(&prompts::merge(&[], true, &notes)) <= allowance);
}
