//! Agent turns through the summary client (KTD4, KTD5): a request with a
//! history and tools, and the whole reply with its tool calls, usage, cost
//! and serving host.

mod common;

use std::collections::{BTreeSet, HashMap};

use common::{Reply, anthropic_reply, chat_reply, serve};
use rig_agent::run::{AgentRun, AgentRunStep, ModelTurn, PendingToolCall, TurnPolicy};
use rig_core::completion::{AssistantContent, AssistantMessage, Message, ToolDefinition};
use rig_core::message::ToolName;
use rig_core::tool::{ToolOutput, ToolResult};
use serde_json::{Value, json};
use summarize::client::{Label, Message as PlainMessage, RequestKind};
use summarize::{AgentTurn, Client, Endpoint, OpenRouterRouting, Provider, SummarizeError};

const GEMINI_PATH: &str = "/v1beta/models/m:generateContent";

fn client(provider: Provider, base: &str) -> Client {
    Client::new(Endpoint {
        provider,
        base_url: base.into(),
        model: "m".into(),
        credential: Some("k-secret".into()),
    })
    .unwrap()
}

fn label() -> Label {
    Label::note("s1", RequestKind::AgentTurn)
}

fn name(text: &str) -> ToolName {
    ToolName::new(text).unwrap()
}

fn get_note() -> ToolDefinition {
    ToolDefinition::new(
        name("get_note"),
        "Read one note.",
        json!({"type": "object", "properties": {"note_id": {"type": "string"}}}),
    )
}

/// A first turn: the rules, the request and one tool.
fn first_turn() -> AgentTurn {
    AgentTurn {
        preamble: "the rules".into(),
        history: Vec::new(),
        prompt: Message::user("write the summary"),
        tools: vec![get_note()],
        max_tokens: 100,
        num_ctx: Some(4096),
    }
}

/// A later turn: the model called get note, and the tool answered.
fn turn_after_a_call() -> AgentTurn {
    let call = AssistantContent::tool_call("call_1", name("get_note"), json!({"note_id": "n1"}));
    let AssistantContent::ToolCall(tool_call) = &call else {
        unreachable!()
    };
    let result = Message::tool_result(tool_call.id.clone(), name("get_note"), "line one");
    AgentTurn {
        history: vec![
            Message::user("write the summary"),
            Message::Assistant(AssistantMessage::new(vec![call])),
        ],
        prompt: result,
        ..first_turn()
    }
}

fn base_for(provider: Provider, mock_base: &str) -> String {
    match provider {
        Provider::Gemini => mock_base.trim_end_matches("/v1").to_string(),
        _ => mock_base.to_string(),
    }
}

fn path_for(provider: Provider) -> &'static str {
    match provider {
        Provider::Anthropic => "/v1/messages",
        Provider::Gemini => GEMINI_PATH,
        Provider::Ollama => "/api/chat",
        _ => "/v1/chat/completions",
    }
}

/// A chat reply with tool calls `(id, name, arguments text)` and no text.
fn chat_tool_reply(calls: &[(&str, &str, &str)]) -> String {
    let calls: Vec<Value> = calls
        .iter()
        .map(|(id, name, arguments)| {
            json!({"id": id, "type": "function", "function": {"name": name, "arguments": arguments}})
        })
        .collect();
    json!({
        "choices": [{
            "message": {"role": "assistant", "content": null, "tool_calls": calls},
            "finish_reason": "tool_calls"
        }]
    })
    .to_string()
}

#[tokio::test]
async fn a_reply_with_two_tool_calls_and_no_text_returns_both_calls() {
    let reply = chat_tool_reply(&[
        ("call_a", "get_note", r#"{"note_id":"n1"}"#),
        ("call_b", "list_notes", r#"{"tags":["x"]}"#),
    ]);
    let mock = serve(HashMap::from([(
        "/v1/chat/completions",
        Reply::Json(200, reply),
    )]))
    .await;
    let reply = client(Provider::Custom, &mock.base)
        .agent_turn(&label(), &first_turn())
        .await
        .unwrap();
    let calls: Vec<_> = reply.response.tool_calls().collect();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].function.name.as_str(), "get_note");
    assert_eq!(calls[0].function.arguments["note_id"], "n1");
    assert_eq!(calls[1].function.name.as_str(), "list_notes");
    assert_eq!(calls[1].function.arguments["tags"], json!(["x"]));
    assert_ne!(calls[0].id, calls[1].id);
    assert!(reply.response.text().is_empty());
}

#[tokio::test]
async fn a_tool_call_with_arguments_that_are_not_json_keeps_them_as_invalid() {
    let reply = chat_tool_reply(&[("call_a", "get_note", "{not json")]);
    let mock = serve(HashMap::from([(
        "/v1/chat/completions",
        Reply::Json(200, reply),
    )]))
    .await;
    let reply = client(Provider::Custom, &mock.base)
        .agent_turn(&label(), &first_turn())
        .await
        .unwrap();
    let call = reply.response.tool_calls().next().unwrap();
    assert_eq!(
        call.function.invalid_arguments.as_deref(),
        Some("{not json")
    );
}

/// Send the turn after a tool call to `provider`, and return the body that
/// the endpoint got.
async fn body_after_a_call(provider: Provider, reply: String) -> Value {
    let mock = serve(HashMap::from([(
        path_for(provider),
        Reply::Json(200, reply),
    )]))
    .await;
    client(provider, &base_for(provider, &mock.base))
        .agent_turn(&label(), &turn_after_a_call())
        .await
        .unwrap();
    mock.seen.lock().unwrap()[0].json()
}

#[tokio::test]
async fn the_openai_compatible_route_sends_the_call_the_result_and_the_tools() {
    let body = body_after_a_call(Provider::Custom, chat_reply("done", "stop")).await;
    let messages = body["messages"].as_array().unwrap();
    let assistant = messages.iter().find(|m| m["role"] == "assistant").unwrap();
    assert_eq!(assistant["tool_calls"][0]["id"], "call_1");
    assert_eq!(assistant["tool_calls"][0]["function"]["name"], "get_note");
    let tool = messages.iter().find(|m| m["role"] == "tool").unwrap();
    assert_eq!(tool["tool_call_id"], "call_1");
    assert_eq!(body["tools"][0]["function"]["name"], "get_note");
    assert_eq!(messages[0]["role"], "system");
}

#[tokio::test]
async fn the_anthropic_route_sends_the_call_the_result_and_the_tools() {
    let body = body_after_a_call(Provider::Anthropic, anthropic_reply("done", "end_turn")).await;
    let text = body.to_string();
    assert!(text.contains(r#""type":"tool_use""#), "{text}");
    assert!(text.contains(r#""type":"tool_result""#), "{text}");
    assert!(text.contains(r#""tool_use_id":"call_1""#), "{text}");
    assert_eq!(body["tools"][0]["name"], "get_note");
    assert!(body["system"].to_string().contains("the rules"));
}

#[tokio::test]
async fn the_gemini_route_sends_the_call_the_result_and_the_tools() {
    let reply = common::gemini_reply("done", "STOP");
    let body = body_after_a_call(Provider::Gemini, reply).await;
    let text = body.to_string();
    assert!(text.contains(r#""functionCall""#), "{text}");
    assert!(text.contains(r#""functionResponse""#), "{text}");
    assert_eq!(
        body["tools"][0]["functionDeclarations"][0]["name"],
        "get_note"
    );
}

#[tokio::test]
async fn the_ollama_route_sends_the_call_the_result_and_the_tools() {
    let reply = common::ollama_reply("done", "stop");
    let body = body_after_a_call(Provider::Ollama, reply).await;
    let messages = body["messages"].as_array().unwrap();
    let assistant = messages.iter().find(|m| m["role"] == "assistant").unwrap();
    assert_eq!(assistant["tool_calls"][0]["function"]["name"], "get_note");
    assert!(messages.iter().any(|m| m["role"] == "tool"));
    assert_eq!(body["tools"][0]["function"]["name"], "get_note");
    assert_eq!(body["options"]["num_ctx"], 4096);
}

#[tokio::test]
async fn the_reply_gives_its_usage_cost_and_serving_host() {
    let reply = json!({
        "id": "gen-1",
        "provider": "SomeHost",
        "choices": [{"message": {"role": "assistant", "content": "done"}, "finish_reason": "stop"}],
        "usage": {
            "prompt_tokens": 1200,
            "completion_tokens": 300,
            "total_tokens": 1500,
            "completion_tokens_details": {"reasoning_tokens": 40},
            "cost": 0.0021
        }
    })
    .to_string();
    let mock = serve(HashMap::from([(
        "/v1/chat/completions",
        Reply::Json(200, reply),
    )]))
    .await;
    let reply = client(Provider::OpenRouter, &mock.base)
        .agent_turn(&label(), &first_turn())
        .await
        .unwrap();
    let usage = reply.response.usage;
    assert_eq!(usage.input_tokens, Some(1200));
    assert_eq!(usage.output_tokens, Some(300));
    assert_eq!(usage.reasoning_tokens, Some(40));
    assert_eq!(reply.cost, Some(0.0021));
    assert_eq!(reply.served_by.as_deref(), Some("SomeHost"));
}

const DENY: OpenRouterRouting = OpenRouterRouting {
    deny_data_collection: true,
    zero_retention: false,
};

fn one_message() -> Vec<PlainMessage> {
    vec![PlainMessage {
        role: "user",
        content: "hello".into(),
    }]
}

#[tokio::test]
async fn openrouter_routing_rules_go_in_the_provider_field() {
    let mock = serve(HashMap::from([(
        "/v1/chat/completions",
        Reply::Json(200, chat_reply("done", "stop")),
    )]))
    .await;
    let with_rules = client(Provider::OpenRouter, &mock.base).with_openrouter(DENY);
    with_rules
        .agent_turn(&label(), &first_turn())
        .await
        .unwrap();
    with_rules
        .complete(&label(), &one_message(), 100, None)
        .await
        .unwrap();
    client(Provider::OpenRouter, &mock.base)
        .complete(&label(), &one_message(), 100, None)
        .await
        .unwrap();
    let seen = mock.seen.lock().unwrap();
    let agent = seen[0].json();
    assert_eq!(agent["provider"]["require_parameters"], true);
    assert_eq!(agent["provider"]["data_collection"], "deny");
    let complete = seen[1].json();
    assert_eq!(complete["provider"]["data_collection"], "deny");
    assert!(complete["provider"].get("require_parameters").is_none());
    let plain = seen[2].json();
    assert!(plain.get("provider").is_none(), "{plain}");
}

#[tokio::test]
async fn lm_studio_never_gets_a_provider_field() {
    let mock = serve(HashMap::from([(
        "/v1/chat/completions",
        Reply::Json(200, chat_reply("done", "stop")),
    )]))
    .await;
    let lm_studio = client(Provider::LmStudio, &mock.base).with_openrouter(DENY);
    lm_studio.agent_turn(&label(), &first_turn()).await.unwrap();
    lm_studio
        .complete(&label(), &one_message(), 100, None)
        .await
        .unwrap();
    let seen = mock.seen.lock().unwrap();
    for request in seen.iter() {
        assert!(request.json().get("provider").is_none());
    }
}

#[tokio::test]
async fn a_gemini_reply_builds_a_model_turn_and_keeps_its_thought_signature() {
    let reply = json!({
        "candidates": [{
            "content": {
                "role": "model",
                "parts": [{
                    "functionCall": {"name": "get_note", "args": {"note_id": "n1"}},
                    "thoughtSignature": "c2lnbmF0dXJl"
                }]
            },
            "finishReason": "STOP"
        }]
    })
    .to_string();
    let mock = serve(HashMap::from([(GEMINI_PATH, Reply::Json(200, reply))])).await;
    let gemini = client(Provider::Gemini, &base_for(Provider::Gemini, &mock.base));
    let mut run = AgentRun::new(Message::user("write the summary")).max_turns(3);
    assert!(matches!(
        run.next_step().unwrap(),
        AgentRunStep::CallModel { .. }
    ));
    let reply = gemini.agent_turn(&label(), &first_turn()).await.unwrap();
    let names = BTreeSet::from(["get_note".to_string()]);
    let policy = TurnPolicy::new(names, None, None).unwrap();
    run.model_response(ModelTurn::from_policy(&reply.response, policy))
        .unwrap();
    let AgentRunStep::CallTools { mut calls } = run.next_step().unwrap() else {
        panic!("the run does not ask for the tool call");
    };
    assert_eq!(calls.len(), 1);
    let Some(PendingToolCall::Execute(call)) = calls.pop() else {
        panic!("the call cannot run");
    };
    run.answer(call.answer(ToolResult::success(ToolOutput::text("line one"))))
        .unwrap();
    let AgentRunStep::CallModel {
        prompt, history, ..
    } = run.next_step().unwrap()
    else {
        panic!("the run does not ask for the next model call");
    };
    let next = AgentTurn {
        history: history.iter().map(|m| (**m).clone()).collect(),
        prompt: (*prompt).clone(),
        ..first_turn()
    };
    let _ = gemini.agent_turn(&label(), &next).await;
    let body = mock.seen.lock().unwrap()[1].json();
    assert!(body.to_string().contains("c2lnbmF0dXJl"), "{body}");
}

#[tokio::test]
async fn a_cut_off_agent_reply_is_reply_cut_off() {
    let mock = serve(HashMap::from([(
        "/v1/chat/completions",
        Reply::Json(200, chat_reply("half", "length")),
    )]))
    .await;
    let error = client(Provider::Custom, &mock.base)
        .agent_turn(&label(), &first_turn())
        .await
        .unwrap_err();
    assert!(matches!(error, SummarizeError::ReplyCutOff), "{error:?}");
}

#[tokio::test]
async fn an_anthropic_context_window_stop_on_an_agent_turn_is_context_exceeded() {
    let mock = serve(HashMap::from([(
        "/v1/messages",
        Reply::Json(
            200,
            anthropic_reply("half", "model_context_window_exceeded"),
        ),
    )]))
    .await;
    let error = client(Provider::Anthropic, &mock.base)
        .agent_turn(&label(), &first_turn())
        .await
        .unwrap_err();
    assert!(
        matches!(error, SummarizeError::ContextExceeded),
        "{error:?}"
    );
}

#[tokio::test]
async fn an_agent_turn_that_the_model_refuses_for_its_reply_cap_goes_again_with_that_cap() {
    let responder: common::Responder = std::sync::Arc::new(|body: &str| {
        let request: Value = serde_json::from_str(body).unwrap();
        if request["max_tokens"] == 8_000 {
            let error = json!({"error": {"message": "max_tokens: 8000 > 4096, which is the maximum allowed"}});
            (400, error.to_string())
        } else {
            (200, chat_reply("done", "stop"))
        }
    });
    let mock = serve(HashMap::from([(
        "/v1/chat/completions",
        Reply::With(responder),
    )]))
    .await;
    let turn = AgentTurn {
        max_tokens: 8_000,
        ..first_turn()
    };
    // The client removes the model name from an error before it reads a
    // cap, so the model name must not occur in the error text.
    Client::new(Endpoint {
        provider: Provider::Custom,
        base_url: mock.base.clone(),
        model: "capped-model".into(),
        credential: Some("k-secret".into()),
    })
    .unwrap()
    .agent_turn(&label(), &turn)
    .await
    .unwrap();
    let seen = mock.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1].json()["max_tokens"], 4_096);
}

/// A client of the model `model` on `base` with the Thinking level Low,
/// chosen by the user or not.
fn thinking_client(base: &str, model: &str, chosen: bool) -> Client {
    Client::new(Endpoint {
        provider: Provider::Custom,
        base_url: base.into(),
        model: model.into(),
        credential: Some("k-secret".into()),
    })
    .unwrap()
    .with_thinking(summarize::Thinking {
        level: summarize::provider::ThinkingLevel::Low,
        chosen,
    })
}

/// A server that refuses each request with a reasoning option.
async fn refusing_reasoning() -> common::Mock {
    let responder: common::Responder = std::sync::Arc::new(|body: &str| {
        let request: Value = serde_json::from_str(body).unwrap();
        if request.get("reasoning_effort").is_some() {
            (
                400,
                r#"{"error":{"message":"reasoning_effort is not supported"}}"#.into(),
            )
        } else {
            (200, chat_reply("A summary.", "stop"))
        }
    });
    serve(HashMap::from([(
        "/v1/chat/completions",
        Reply::With(responder),
    )]))
    .await
}

/// An automatic Thinking level that the model refuses: the agent turn
/// goes again with no reasoning option, as a completion does. The model
/// name is unique, because the client keeps the refusal for the model.
#[tokio::test]
async fn a_refused_automatic_thinking_level_sends_the_agent_turn_again_with_no_reasoning() {
    let mock = refusing_reasoning().await;
    thinking_client(&mock.base, "automatic-thinking-model", false)
        .agent_turn(&label(), &first_turn())
        .await
        .unwrap();
    let seen = mock.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].json()["reasoning_effort"], "low");
    assert!(seen[1].json().get("reasoning_effort").is_none());
}

/// A Thinking level that the user chose is never dropped: its refusal is
/// `ReasoningRefused`, and the turn is not sent again.
#[tokio::test]
async fn a_refused_chosen_thinking_level_fails_the_agent_turn() {
    let mock = refusing_reasoning().await;
    let error = thinking_client(&mock.base, "chosen-thinking-model", true)
        .agent_turn(&label(), &first_turn())
        .await
        .unwrap_err();
    assert!(
        matches!(error, SummarizeError::ReasoningRefused { sent: true }),
        "{error:?}"
    );
    assert_eq!(mock.seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn complete_still_refuses_an_empty_reply() {
    let mock = serve(HashMap::from([(
        "/v1/chat/completions",
        Reply::Json(200, chat_tool_reply(&[("call_a", "get_note", "{}")])),
    )]))
    .await;
    let error = client(Provider::Custom, &mock.base)
        .complete(&label(), &one_message(), 100, None)
        .await
        .unwrap_err();
    assert!(matches!(error, SummarizeError::Malformed(_)), "{error:?}");
}
