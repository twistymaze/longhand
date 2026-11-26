//! The raw summary request body of each provider preset is the same, byte
//! for byte, as the body that rig-core 0.44 sent. The fixtures in
//! `golden/request-bodies/` hold those bodies. A dependency update that
//! changes a field, a value or the key order fails here.

mod common;

use std::collections::HashMap;

use common::{Reply, anthropic_reply, chat_reply, gemini_reply, ollama_reply, serve};
use summarize::client::{Label, Message, RequestKind};
use summarize::{Client, Endpoint, Provider};

/// The raw body of one summary request for `provider`, sent to a server
/// that answers on `path` with `reply`.
async fn body_of(provider: Provider, path: &'static str, reply: String) -> String {
    let mock = serve(HashMap::from([(path, Reply::Json(200, reply))])).await;
    let base = match provider {
        // Gemini's paths start at the server root.
        Provider::Gemini => mock.base.trim_end_matches("/v1").to_string(),
        _ => mock.base.clone(),
    };
    let endpoint = Endpoint {
        provider,
        base_url: base,
        model: "m".into(),
        credential: Some("k".into()),
    };
    let messages = vec![
        Message {
            role: "system",
            content: "the rules".into(),
        },
        Message {
            role: "user",
            content: "hello".into(),
        },
    ];
    Client::new(endpoint)
        .unwrap()
        .complete(
            &Label::note("s1", RequestKind::Writer),
            &messages,
            100,
            Some(4096),
        )
        .await
        .unwrap();
    let seen = mock.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    seen[0].body.clone()
}

#[tokio::test]
async fn each_preset_sends_the_same_body_as_before() {
    let cases = [
        (
            Provider::Custom,
            "/v1/chat/completions",
            chat_reply("fine", "stop"),
            include_str!("golden/request-bodies/custom.json"),
        ),
        (
            Provider::OpenAi,
            "/v1/chat/completions",
            chat_reply("fine", "stop"),
            include_str!("golden/request-bodies/openai.json"),
        ),
        (
            Provider::OpenRouter,
            "/v1/chat/completions",
            chat_reply("fine", "stop"),
            include_str!("golden/request-bodies/openrouter.json"),
        ),
        (
            Provider::LmStudio,
            "/v1/chat/completions",
            chat_reply("fine", "stop"),
            include_str!("golden/request-bodies/lm-studio.json"),
        ),
        (
            Provider::Anthropic,
            "/v1/messages",
            anthropic_reply("fine", "end_turn"),
            include_str!("golden/request-bodies/anthropic.json"),
        ),
        (
            Provider::Gemini,
            "/v1beta/models/m:generateContent",
            gemini_reply("fine", "STOP"),
            include_str!("golden/request-bodies/gemini.json"),
        ),
        (
            Provider::Ollama,
            "/api/chat",
            ollama_reply("fine", "stop"),
            include_str!("golden/request-bodies/ollama.json"),
        ),
    ];
    for (provider, path, reply, before) in cases {
        let body = body_of(provider, path, reply).await;
        assert_eq!(body, before, "{provider:?}");
    }
}
