//! The prompts of the summary agent (U4; R12, R13, AE7, KTD13).

use summarize::chunk::{Chunk, Line, Turn};
use summarize::prompts::{
    AGENT_DATA_RULE, AgentFacts, DEFAULT_STRATEGY, Details, NAMING_RULE, UserText, agent, reader,
    step_merge, step_part,
};

fn facts() -> AgentFacts {
    AgentFacts {
        note_id: "note-budget-review".into(),
        transcript_lines: 812,
        transcript_chars: 61_000,
        has_manual_notes: true,
    }
}

fn details() -> Details {
    Details {
        note_type: Some("Meetings".into()),
        tags: vec!["Person::Sam".into()],
        title: Some("Budget review".into()),
    }
}

#[test]
fn the_agent_prompt_has_the_details_the_background_and_rules_and_no_transcript() {
    let details = details();
    let user = UserText {
        background: "I work with Sam on the budget.",
        details: &details,
    };
    let messages = agent(
        &facts(),
        DEFAULT_STRATEGY,
        "Write a short summary.",
        None,
        user,
    );
    assert_eq!(messages.len(), 2);
    let (system, first) = (&messages[0], &messages[1]);
    assert_eq!(system.role, "system");
    assert!(system.content.contains("I work with Sam on the budget."));
    assert!(system.content.contains("Write a short summary."));
    assert!(
        system
            .content
            .contains("Never label a person \"Speaker 1\"")
    );
    assert!(system.content.contains(DEFAULT_STRATEGY));
    assert!(first.content.contains("<session_details>"));
    assert!(first.content.contains("Person::Sam"));
    assert!(first.content.contains("note-budget-review"));
    assert!(first.content.contains("812 lines"));
    assert!(!first.content.contains("<transcript"));
    assert!(!system.content.contains("read_part"));
}

#[test]
fn the_agent_prompt_has_the_tool_result_data_rule_twice_and_it_names_other_notes() {
    let messages = agent(&facts(), "", "Summary text.", None, "");
    assert!(AGENT_DATA_RULE.contains("also when they come from another note"));
    assert!(AGENT_DATA_RULE.contains("list the notes tagged"));
    assert!(messages[0].content.contains(AGENT_DATA_RULE));
    assert!(messages[1].content.ends_with(AGENT_DATA_RULE));
    // The naming rule stays last in the system text, after the prompt text.
    assert!(messages[0].content.ends_with(NAMING_RULE));
}

#[test]
fn the_reading_strategy_goes_where_the_harness_puts_it() {
    let strategy = "Read every third page only.";
    let messages = agent(&facts(), strategy, "Summary text.", None, "");
    let system = &messages[0].content;
    let at = system.find(strategy).unwrap();
    assert!(at < system.find("Summary text.").unwrap());
    assert!(at > system.find(AGENT_DATA_RULE).unwrap());
}

#[test]
fn a_lead_prompt_names_read_part_and_the_largest_part() {
    let messages = agent(&facts(), "", "Summary text.", Some(240), "");
    assert!(messages[0].content.contains("read_part"));
    assert!(messages[0].content.contains("at most 240 lines"));
}

#[test]
fn the_reader_prompt_has_its_lines_size_and_instruction_and_no_tools() {
    let lines = "[00:04:01] Speaker 1: The budget is 40k.";
    let messages = reader(
        241,
        480,
        lines,
        "Note every decision about money.",
        Some(300),
        "",
    );
    let all = format!("{}\n{}", messages[0].content, messages[1].content);
    assert!(all.contains("lines 241 to 480"));
    assert!(all.contains("<transcript lines=\"241-480\">"));
    assert!(all.contains("The budget is 40k."));
    assert!(all.contains("at most about 300 words"));
    assert!(all.contains("Note every decision about money."));
    assert!(all.contains(AGENT_DATA_RULE));
    for tool in ["read_part", "list_notes", "get_note"] {
        assert!(!all.contains(tool), "the reader prompt names {tool}");
    }
}

#[test]
fn a_step_part_prompt_holds_its_part_and_the_merge_prompt_no_transcript() {
    let chunk = Chunk {
        lines: vec![Line::Turn(Turn {
            position_ms: 61_000,
            speaker: Some(1),
            text: "The travel line is lower.".into(),
        })],
        first_new: 0,
    };
    let part = step_part(&chunk, 0, 3, "note-x", "");
    assert!(part[1].content.contains("<transcript part=\"1\">"));
    assert!(part[1].content.contains("The travel line is lower."));
    assert!(part[0].content.contains("get_note"));
    let merge = step_merge(&["- [00:01:01] notes"], "note-x", "Summary text.", "");
    assert!(!merge[1].content.contains("<transcript"));
    assert!(merge[0].content.contains("Summary text."));
    assert!(merge[0].content.contains("get_note"));
}
