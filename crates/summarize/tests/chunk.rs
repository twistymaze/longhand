//! Packing whole turns under a budget.

use summarize::chunk::{Chunk, Note, Turn, chunk, render_turn};
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
        + chunk
            .notes
            .iter()
            .map(|n| estimate(&summarize::chunk::render_note(n)) + 1)
            .sum::<usize>()
}

#[test]
fn a_four_hour_transcript_packs_under_a_small_budget_without_splitting_a_turn() {
    let all = turns(40_000);
    let allowance = prompt_allowance(4_096, reply_reserve(4_096)) - 400;
    let chunks = chunk(&all, &[], allowance);
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
fn each_note_travels_with_the_one_chunk_that_owns_its_moment() {
    let all = turns(40_000);
    let notes = [
        Note {
            position_ms: Some(3_600_000),
            text: "the decision".into(),
        },
        Note {
            position_ms: Some(3_630_000),
            text: "and why".into(),
        },
    ];
    let chunks = chunk(&all, &notes, 2_000);
    for note in &notes {
        let holding: Vec<&Chunk> = chunks.iter().filter(|c| c.notes.contains(note)).collect();
        assert_eq!(holding.len(), 1, "{note:?} is in {} chunks", holding.len());
        let owned = &holding[0].turns[holding[0].first_new..];
        let at = note.position_ms.unwrap();
        assert!(owned.first().unwrap().position_ms <= at);
        assert!(owned.last().unwrap().position_ms + 360 > at);
    }
    // Overlap repeats turns, never notes.
    assert_eq!(chunks.iter().map(|c| c.notes.len()).sum::<usize>(), 2);
}

#[test]
fn one_turn_longer_than_a_chunk_splits_at_sentences_keeping_label_and_time() {
    let sentence = "This is one sentence of a very long monologue that goes on. ";
    let long = Turn {
        position_ms: 7_200_000,
        speaker: Some(3),
        text: sentence.repeat(200).trim().to_string(),
    };
    let chunks = chunk(std::slice::from_ref(&long), &[], 300);
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

#[test]
fn a_gap_anchored_note_goes_with_the_segment_before_the_gap() {
    let all = vec![
        Turn {
            position_ms: 0,
            speaker: Some(1),
            text: "before the break".into(),
        },
        Turn {
            position_ms: 600_000,
            speaker: Some(1),
            text: "after the break".into(),
        },
    ];
    // Typed during the gap: anchored at the first segment's end, 300 s.
    let notes = [Note {
        position_ms: Some(300_000),
        text: "break".into(),
    }];
    let chunks = chunk(&all, &notes, 12);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].notes.len(), 1);
    assert!(chunks[1].notes.is_empty());
}
