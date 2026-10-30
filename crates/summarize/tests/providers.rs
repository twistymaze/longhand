//! Summary requests through Rig for each provider (R1–R4, R6; KTD2, KTD3,
//! KTD4, KTD7): the request that each route sends, and the map from each
//! route's reply and errors to the Longhand error kinds.

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{Reply, anthropic_reply, chat_reply, closed_base, gemini_reply, ollama_reply, serve};
use serde_json::{Value, json};
use summarize::chunk::{Line, Turn};
use summarize::client::{Label, Message, RequestKind};
use summarize::{Client, Endpoint, Limits, Provider, SummarizeError, prompts};

const GEMINI_PATH: &str = "/v1beta/models/m:generateContent?key=k-secret";

fn endpoint(provider: Provider, base: &str, key: Option<&str>) -> Endpoint {
    Endpoint {
        provider,
        base_url: base.into(),
        model: "m".into(),
        credential: key.map(str::to_string),
    }
}

fn client(provider: Provider, base: &str, key: Option<&str>) -> Client {
    Client::new(endpoint(provider, base, key)).unwrap()
}

fn label() -> Label {
    Label::note("s1", RequestKind::Single)
}

fn sys_and_user() -> Vec<Message> {
    vec![
        Message {
            role: "system",
            content: "the rules".into(),
        },
        Message {
            role: "user",
            content: "hello".into(),
        },
    ]
}

/// The Gemini root: the mock base without its `/v1`.
fn root(base: &str) -> String {
    base.trim_end_matches("/v1").to_string()
}

/// The base for `provider` on `mock_base`: Gemini's paths start at the
/// server root.
fn base_for(provider: Provider, mock_base: &str) -> String {
    match provider {
        Provider::Gemini => root(mock_base),
        _ => mock_base.to_string(),
    }
}

/// The path that `provider` sends a summary request to.
fn path_for(provider: Provider) -> &'static str {
    match provider {
        Provider::Anthropic => "/v1/messages",
        Provider::Gemini => GEMINI_PATH,
        Provider::Ollama => "/api/chat",
        _ => "/v1/chat/completions",
    }
}

/// Send one summary request for `provider` to a server that answers
/// `reply`, and return the result.
async fn complete_with(provider: Provider, reply: Reply) -> Result<String, SummarizeError> {
    let mock = serve(HashMap::from([(path_for(provider), reply)])).await;
    client(provider, &base_for(provider, &mock.base), Some("k-secret"))
        .complete(&label(), &sys_and_user(), 100, Some(4096))
        .await
        .map(|c| c.text)
}

fn one_turn() -> Vec<Line> {
    vec![
        Turn {
            position_ms: 0,
            speaker: Some(1),
            text: "We agreed on the budget.".into(),
        }
        .into(),
    ]
}

/// The text of one message in a Chat Completions body. Rig can send the
/// content as a string or as an array of text parts.
fn text_of(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .map(|part| {
                assert_eq!(part["type"], "text", "{part}");
                part["text"].as_str().unwrap().to_string()
            })
            .collect(),
        other => panic!("no message text in {other}"),
    }
}

/// Assert that the Chat Completions `body` that Rig built holds `model`,
/// each of `messages` with its role and text, and the reply cap `cap` as
/// `max_tokens` (R9, R11). Longhand writes no `stream` field.
fn assert_chat_body(body: &Value, model: &str, messages: &[Message], cap: u32) {
    assert_eq!(body["model"], model, "{body}");
    let sent = body["messages"].as_array().unwrap();
    assert_eq!(sent.len(), messages.len(), "{body}");
    for (sent, message) in sent.iter().zip(messages) {
        assert_eq!(sent["role"], message.role, "{body}");
        assert_eq!(text_of(&sent["content"]), message.content, "{body}");
    }
    assert_eq!(body["max_tokens"], cap, "{body}");
    assert!(body.get("max_completion_tokens").is_none(), "{body}");
    assert!(body.get("stream").is_none(), "{body}");
}

/// Covers AE7.
#[tokio::test]
async fn custom_sends_a_chat_completions_body_built_by_rig_with_the_bearer_key() {
    let mock = serve(HashMap::from([(
        "/v1/chat/completions",
        Reply::Json(200, chat_reply("fine", "stop")),
    )]))
    .await;
    let topic = prompts::topic_summary();
    let whole = prompts::whole(&one_turn(), "# Notes\n", &topic.text, "");
    let title = prompts::title_turn(&whole, "A summary.", &topic.title_text, "");
    let client = client(Provider::Custom, &mock.base, Some("sk-test"));
    for messages in [&whole, &title] {
        client
            .complete(&label(), messages, 900, None)
            .await
            .unwrap();
    }
    let seen = mock.seen.lock().unwrap();
    for (sent, messages) in seen.iter().zip([&whole, &title]) {
        assert_eq!(sent.method, "POST");
        assert_eq!(sent.path, "/v1/chat/completions");
        assert_eq!(sent.authorization.as_deref(), Some("Bearer sk-test"));
        assert_chat_body(&sent.json(), "m", messages, 900);
    }
}

#[tokio::test]
async fn a_custom_model_id_that_looks_like_a_reasoning_model_still_sends_max_tokens() {
    let mock = serve(HashMap::from([(
        "/v1/chat/completions",
        Reply::Json(200, chat_reply("fine", "stop")),
    )]))
    .await;
    let mut endpoint = endpoint(Provider::Custom, &mock.base, None);
    endpoint.model = "gpt-5-mini".into();
    Client::new(endpoint)
        .unwrap()
        .complete(&label(), &sys_and_user(), 100, None)
        .await
        .unwrap();
    let body = mock.seen.lock().unwrap()[0].json();
    assert_eq!(body["max_tokens"], 100);
    assert!(body.get("max_completion_tokens").is_none(), "{body}");
}

#[tokio::test]
async fn a_custom_reply_marked_truncated_is_context_pressure() {
    // llama.cpp cuts the prompt to fit, then says so.
    let reply = json!({
        "truncated": true,
        "choices": [{"message": {"content": "part"}, "finish_reason": "stop"}]
    });
    let err = complete_with(Provider::Custom, Reply::Json(200, reply.to_string()))
        .await
        .unwrap_err();
    assert!(err.is_context_pressure(), "{err:?}");
}

/// Text that only Rig, reqwest or the request URL can put in an error.
fn assert_longhand_text(err: &SummarizeError) {
    let text = err.to_string();
    for forbidden in [
        "key=",
        "k-secret",
        "/v1beta",
        "generateContent",
        "ProviderResponseError",
        "ResponseError",
        "HttpError",
        "JsonError",
        "RequestError",
    ] {
        assert!(!text.contains(forbidden), "{forbidden:?} in {text:?}");
    }
}

#[tokio::test]
async fn gemini_errors_hold_no_key_no_url_path_and_no_rig_text() {
    let malformed = complete_with(
        Provider::Gemini,
        Reply::Json(200, r#"{"nothing":1}"#.into()),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(malformed, SummarizeError::Malformed(_)),
        "{malformed:?}"
    );
    assert_longhand_text(&malformed);

    let failed = complete_with(
        Provider::Gemini,
        Reply::Json(
            500,
            r#"{"error":{"code":500,"message":"Internal error","status":"INTERNAL"}}"#.into(),
        ),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(failed, SummarizeError::Http { status: 500, .. }),
        "{failed:?}"
    );
    assert_longhand_text(&failed);

    let unreachable = client(Provider::Gemini, &root(&closed_base()), Some("k-secret"))
        .complete(&label(), &sys_and_user(), 100, None)
        .await
        .unwrap_err();
    assert!(
        matches!(unreachable, SummarizeError::Transport(_)),
        "{unreachable:?}"
    );
    assert_longhand_text(&unreachable);
}

#[tokio::test]
async fn the_openai_preset_sends_chat_completions_not_responses() {
    let mock = serve(HashMap::from([(
        "/v1/chat/completions",
        Reply::Json(200, chat_reply("fine", "stop")),
    )]))
    .await;
    let reply = client(Provider::OpenAi, &mock.base, Some("sk-test"))
        .complete(&label(), &sys_and_user(), 100, None)
        .await
        .unwrap();
    assert_eq!(reply.text, "fine");
    let seen = mock.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].path, "/v1/chat/completions");
    assert_eq!(seen[0].authorization.as_deref(), Some("Bearer sk-test"));
    assert_chat_body(&seen[0].json(), "m", &sys_and_user(), 100);
}

#[tokio::test]
async fn anthropic_sends_messages_with_its_key_header_a_reply_cap_and_the_system_field() {
    let mock = serve(HashMap::from([(
        "/v1/messages",
        Reply::Json(200, anthropic_reply("fine", "end_turn")),
    )]))
    .await;
    let reply = client(Provider::Anthropic, &mock.base, Some("k-secret"))
        .complete(&label(), &sys_and_user(), 100, Some(4096))
        .await
        .unwrap();
    assert_eq!(reply.text, "fine");
    let seen = mock.seen.lock().unwrap();
    assert_eq!(seen[0].path, "/v1/messages");
    assert_eq!(seen[0].header("x-api-key"), Some("k-secret"));
    assert_eq!(seen[0].authorization, None);
    let body = seen[0].json();
    assert_eq!(body["max_tokens"], 100);
    assert_eq!(body["system"][0]["text"], "the rules", "{body}");
    assert_eq!(body["messages"].as_array().unwrap().len(), 1, "{body}");
    assert_eq!(body["messages"][0]["role"], "user");
    assert!(body.get("num_ctx").is_none() && body.get("options").is_none());
}

#[tokio::test]
async fn gemini_sends_the_key_where_rig_puts_it_and_a_refused_key_is_unauthorized() {
    let mock = serve(HashMap::from([(
        GEMINI_PATH,
        Reply::Json(200, gemini_reply("fine", "STOP")),
    )]))
    .await;
    let reply = client(Provider::Gemini, &root(&mock.base), Some("k-secret"))
        .complete(&label(), &sys_and_user(), 100, None)
        .await
        .unwrap();
    assert_eq!(reply.text, "fine");
    let sent = mock.seen.lock().unwrap()[0].clone();
    assert_eq!(sent.path, GEMINI_PATH);
    assert_eq!(sent.json()["generationConfig"]["maxOutputTokens"], 100);

    let refused = json!({"error": {
        "code": 400,
        "message": "API key not valid. Please pass a valid API key.",
        "status": "INVALID_ARGUMENT"
    }});
    let err = complete_with(Provider::Gemini, Reply::Json(400, refused.to_string()))
        .await
        .unwrap_err();
    assert!(matches!(err, SummarizeError::Unauthorized(400)), "{err:?}");
}

#[tokio::test]
async fn ollama_sends_api_chat_with_the_num_ctx_of_the_call_and_truncate_false() {
    let mock = serve(HashMap::from([(
        "/api/chat",
        Reply::Json(200, ollama_reply("fine", "stop")),
    )]))
    .await;
    let client = client(Provider::Ollama, &mock.base, None);
    let reply = client
        .complete(&label(), &sys_and_user(), 100, Some(8192))
        .await
        .unwrap();
    assert_eq!(reply.text, "fine");
    client
        .complete(&label(), &sys_and_user(), 100, None)
        .await
        .unwrap();
    let seen = mock.seen.lock().unwrap();
    assert_eq!(seen[0].path, "/api/chat");
    assert_eq!(seen[0].authorization, None);
    let body = seen[0].json();
    assert_eq!(body["options"]["num_ctx"], 8192, "{body}");
    assert_eq!(body["options"]["num_predict"], 100, "{body}");
    assert_eq!(body["truncate"], false, "{body}");
    assert_eq!(body["stream"], false, "{body}");
    let without = seen[1].json();
    assert!(without["options"].get("num_ctx").is_none(), "{without}");
    assert_eq!(without["truncate"], false, "{without}");
}

/// One row of the KTD4 table for each route: a cut-off reply.
#[tokio::test]
async fn a_cut_off_reply_is_reply_cut_off_on_each_route() {
    let cases = [
        (Provider::Custom, chat_reply("half", "length")),
        (Provider::OpenAi, chat_reply("half", "length")),
        (Provider::OpenRouter, chat_reply("half", "length")),
        (Provider::LmStudio, chat_reply("half", "length")),
        (Provider::Anthropic, anthropic_reply("half", "max_tokens")),
        (Provider::Gemini, gemini_reply("half", "MAX_TOKENS")),
        (Provider::Ollama, ollama_reply("half", "length")),
    ];
    for (provider, reply) in cases {
        let err = complete_with(provider, Reply::Json(200, reply))
            .await
            .unwrap_err();
        assert!(
            matches!(err, SummarizeError::ReplyCutOff),
            "{provider:?}: {err:?}"
        );
        assert!(!err.is_context_pressure(), "{provider:?}");
    }
}

/// One row of the KTD4 table for each route: a prompt that is too long.
#[tokio::test]
async fn a_context_error_is_context_pressure_on_each_route() {
    let openai = r#"{"error":{"message":"This model's maximum context length is 8192 tokens"}}"#;
    let anthropic = json!({"type": "error", "error": {
        "type": "invalid_request_error",
        "message": "prompt is too long: 210000 tokens > 200000 maximum"
    }});
    let gemini = json!({"error": {
        "code": 400,
        "message": "The input token count (1200000) exceeds the maximum number of tokens allowed (1048576).",
        "status": "INVALID_ARGUMENT"
    }});
    let ollama = json!({"error": "the input length exceeds the context length"});
    let cases = [
        (Provider::Custom, openai.to_string()),
        (Provider::Anthropic, anthropic.to_string()),
        (Provider::Gemini, gemini.to_string()),
        (Provider::Ollama, ollama.to_string()),
    ];
    for (provider, body) in cases {
        let err = complete_with(provider, Reply::Json(400, body))
            .await
            .unwrap_err();
        assert!(err.is_context_pressure(), "{provider:?}: {err:?}");
    }
}

/// One row of the KTD4 table for each route: a refused key.
#[tokio::test]
async fn a_refused_key_is_unauthorized_on_each_route() {
    let gemini = json!({"error": {
        "code": 403,
        "message": "Permission denied",
        "status": "PERMISSION_DENIED",
        "details": [{"reason": "API_KEY_INVALID"}]
    }});
    let cases = [
        (Provider::Custom, 401, r#"{"error":"bad key"}"#.to_string()),
        (
            Provider::OpenRouter,
            403,
            r#"{"error":"bad key"}"#.to_string(),
        ),
        (Provider::Anthropic, 401, r#"{"type":"error"}"#.to_string()),
        (Provider::Gemini, 403, gemini.to_string()),
    ];
    for (provider, status, body) in cases {
        let err = complete_with(provider, Reply::Json(status, body))
            .await
            .unwrap_err();
        assert!(
            matches!(err, SummarizeError::Unauthorized(s) if s == status),
            "{provider:?}: {err:?}"
        );
    }
    // A Gemini 403 that does not name the key is not a key problem.
    let other = json!({"error": {"code": 403, "message": "The API is not enabled"}});
    let err = complete_with(Provider::Gemini, Reply::Json(403, other.to_string()))
        .await
        .unwrap_err();
    assert!(
        matches!(err, SummarizeError::Http { status: 403, .. }),
        "{err:?}"
    );
}

#[tokio::test]
async fn an_anthropic_context_window_stop_is_context_pressure_not_a_cut_off_reply() {
    let err = complete_with(
        Provider::Anthropic,
        Reply::Json(
            200,
            anthropic_reply("part", "model_context_window_exceeded"),
        ),
    )
    .await
    .unwrap_err();
    assert!(err.is_context_pressure(), "{err:?}");
}

#[tokio::test]
async fn a_refused_max_tokens_is_sent_once_more_with_max_completion_tokens() {
    let reply = Reply::With(Arc::new(|body: &str| {
        if body.contains("\"max_tokens\"") {
            (
                400,
                r#"{"error":{"message":"Unsupported parameter: 'max_tokens'. Use 'max_completion_tokens' instead."}}"#.into(),
            )
        } else {
            (200, chat_reply("ok", "stop"))
        }
    }));
    let mock = serve(HashMap::from([("/v1/chat/completions", reply)])).await;
    let text = client(Provider::Custom, &mock.base, None)
        .complete(&label(), &sys_and_user(), 50, None)
        .await
        .unwrap()
        .text;
    assert_eq!(text, "ok");
    let seen = mock.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    let second = seen[1].json();
    assert_eq!(second["max_completion_tokens"], 50, "{second}");
    assert!(second.get("max_tokens").is_none(), "{second}");
    assert!(second.get("stream").is_none(), "{second}");
}

#[tokio::test]
async fn a_request_refused_again_for_max_completion_tokens_is_not_sent_a_third_time() {
    let mock = serve(HashMap::from([(
        "/v1/chat/completions",
        Reply::Json(
            400,
            r#"{"error":{"message":"Unsupported parameter: 'max_tokens'. Use 'max_completion_tokens' instead."}}"#.into(),
        ),
    )]))
    .await;
    let err = client(Provider::Custom, &mock.base, None)
        .complete(&label(), &sys_and_user(), 50, None)
        .await
        .unwrap_err();
    assert!(
        matches!(err, SummarizeError::Http { status: 400, .. }),
        "{err:?}"
    );
    let seen = mock.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1].json()["max_completion_tokens"], 50);
}

#[tokio::test]
async fn a_native_route_sends_a_failed_request_once_and_an_unreachable_one_after_each_wait() {
    let mock = serve(HashMap::from([(
        "/v1/messages",
        Reply::Json(500, r#"{"type":"error"}"#.into()),
    )]))
    .await;
    let err = client(Provider::Anthropic, &mock.base, Some("k"))
        .with_retry_waits(vec![Duration::from_millis(10); 3])
        .complete(&label(), &sys_and_user(), 100, None)
        .await
        .unwrap_err();
    assert!(
        matches!(err, SummarizeError::Http { status: 500, .. }),
        "{err:?}"
    );
    assert_eq!(mock.seen.lock().unwrap().len(), 1);

    // Each try fails to connect. The request ends only after all the
    // waits, and then names the connection failure.
    let waits = vec![Duration::from_millis(40); 3];
    let total: Duration = waits.iter().sum();
    let started = Instant::now();
    let err = client(Provider::Anthropic, &closed_base(), Some("k"))
        .with_retry_waits(waits)
        .complete(&label(), &sys_and_user(), 100, None)
        .await
        .unwrap_err();
    assert!(matches!(err, SummarizeError::Transport(_)), "{err:?}");
    assert!(started.elapsed() >= total, "{:?}", started.elapsed());
}

#[tokio::test]
async fn a_request_past_the_total_deadline_fails_with_the_deadline_error() {
    // The server never answers. The other deadlines are longer than the
    // total deadline, and so is the wait of `Reply::Hang`.
    let mock = serve(HashMap::from([("/v1/messages", Reply::Hang)])).await;
    let long = Duration::from_secs(2 * 60 * 60);
    let limits = Limits {
        connect: long,
        response: long,
        idle: long,
        total: Duration::from_secs(30 * 60),
        max_bytes: 1 << 20,
    };
    let client = Client::with_limits(
        endpoint(Provider::Anthropic, &mock.base, Some("k")),
        limits,
        limits,
    )
    .unwrap()
    .with_retry_waits(vec![Duration::from_millis(10); 3]);
    let (label, messages) = (label(), sys_and_user());
    let complete = client.complete(&label, &messages, 100, None);
    tokio::pin!(complete);
    // The clock runs until the server has read the request. Then it stops,
    // and the runtime moves it to the next timer when no task can run, so
    // the total deadline ends the request.
    tokio::select! {
        () = mock.requests(1) => {}
        result = &mut complete => panic!("the request ended before the server read it: {result:?}"),
    }
    tokio::time::pause();
    let err = complete.await.unwrap_err();
    assert!(
        matches!(err, SummarizeError::Deadline("complete reply")),
        "{err:?}"
    );
    assert_eq!(mock.seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn lm_studio_and_custom_with_no_saved_key_send_no_authorization_header() {
    let mock = serve(HashMap::from([(
        "/v1/chat/completions",
        Reply::Json(200, chat_reply("fine", "stop")),
    )]))
    .await;
    for provider in [Provider::LmStudio, Provider::Custom] {
        for key in [None, Some("")] {
            client(provider, &mock.base, key)
                .complete(&label(), &sys_and_user(), 100, None)
                .await
                .unwrap();
        }
    }
    let seen = mock.seen.lock().unwrap();
    assert_eq!(seen.len(), 4);
    for sent in seen.iter() {
        assert_eq!(sent.authorization, None, "{}", sent.body);
        assert_chat_body(&sent.json(), "m", &sys_and_user(), 100);
    }
}

#[test]
fn an_empty_address_is_the_default_address_of_the_provider() {
    let host = |provider| client(provider, "", None).host();
    assert_eq!(host(Provider::OpenAi), "api.openai.com");
    assert_eq!(host(Provider::Anthropic), "api.anthropic.com");
    assert_eq!(host(Provider::Gemini), "generativelanguage.googleapis.com");
    assert_eq!(host(Provider::OpenRouter), "openrouter.ai");
    assert_eq!(host(Provider::Ollama), "localhost");
    assert_eq!(host(Provider::LmStudio), "localhost");
    assert!(matches!(
        Client::new(endpoint(Provider::Custom, " ", None)),
        Err(SummarizeError::BadUrl(_))
    ));
    // A typed address wins over the default address.
    assert_eq!(
        client(Provider::Ollama, "http://gpu-box.local:11434", None).host(),
        "gpu-box.local"
    );
}
