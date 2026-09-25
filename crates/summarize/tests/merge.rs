//! Layered merging of chunk summaries.

use summarize::chunk::merge_groups;

#[test]
fn nine_summaries_merge_in_layers_to_one_with_every_call_under_budget() {
    let allowance = 1_000;
    let mut costs = vec![300usize; 9];
    let mut layers = 0;
    while costs.len() > 1 {
        let groups = merge_groups(&costs, allowance);
        assert!(groups.len() < costs.len(), "each layer shrinks");
        for g in &groups {
            let total: usize = costs[g.clone()].iter().sum();
            assert!(g.len() == 1 || total <= allowance, "{g:?} costs {total}");
        }
        // A merged summary costs about as much as one part.
        costs = groups.iter().map(|_| 300).collect();
        layers += 1;
    }
    assert_eq!(layers, 2);
}

#[test]
fn a_lone_summary_at_the_end_of_a_layer_passes_up_unchanged() {
    let groups = merge_groups(&[400, 400, 400], 800);
    assert_eq!(groups, vec![0..2, 2..3]);
}

/// Covers AE9: grouping reserves the room for the prompt text in every
/// layer, because it does not know which layer is last.
#[test]
fn with_a_prompt_text_at_the_limit_every_merge_call_stays_under_budget() {
    use summarize::prompts::{self, PROMPT_TEXT_LIMIT};
    use summarize::tokens::{estimate, estimate_messages, prompt_allowance, reply_reserve};

    let text = "word ".repeat(PROMPT_TEXT_LIMIT / 5);
    let notes = "- the loot\n- the mist\n";
    let limit = 8_192;
    let budget = prompt_allowance(limit, reply_reserve(limit));
    let allowance = budget - prompts::overhead(notes, &text);
    // Nine part notes of about 900 tokens each.
    let mut summaries: Vec<String> = (0..9)
        .map(|i| format!("{i} {}", "x".repeat(3_150)))
        .collect();
    let mut layers = 0;
    while summaries.len() > 1 {
        let costs: Vec<usize> = summaries.iter().map(|s| estimate(s) + 8).collect();
        let groups = merge_groups(&costs, allowance);
        let last = groups.len() == 1;
        let mut next = Vec::new();
        for g in &groups {
            if g.len() == 1 {
                next.push(summaries[g.start].clone());
                continue;
            }
            let parts: Vec<&str> = summaries[g.clone()].iter().map(String::as_str).collect();
            let messages = prompts::merge(&parts, last, notes, &text);
            let cost = estimate_messages(messages.iter().map(|m| m.content.as_str()));
            assert!(cost <= budget, "layer {layers}: {cost} of {budget}");
            assert_eq!(messages[0].content.contains(text.trim()), last);
            next.push(parts[0].to_string());
        }
        summaries = next;
        layers += 1;
    }
    assert!(layers >= 2, "{layers}");
}
