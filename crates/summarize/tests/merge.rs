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
