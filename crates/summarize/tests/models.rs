//! The model list: the list shapes, and each outcome of the list request.

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{Reply, serve};
use serde_json::json;
use summarize::models::parse_ids;
use summarize::{Checked, Client, Endpoint, Limits, ModelList, Provider, check, list_models};

fn endpoint(base: &str, credential: Option<&str>) -> Endpoint {
    Endpoint {
        provider: Provider::Custom,
        base_url: base.into(),
        model: String::new(),
        credential: credential.map(str::to_string),
    }
}

/// Serves `reply` at `/v1/models` and lists the models with `credential`.
async fn list_with(reply: Reply, credential: Option<&str>) -> ModelList {
    let mock = serve(HashMap::from([("/v1/models", reply)])).await;
    list_models(&Client::new(endpoint(&mock.base, credential)).unwrap()).await
}

fn listed(ids: &[&str]) -> ModelList {
    ModelList::Listed {
        ids: ids.iter().map(|s| s.to_string()).collect(),
    }
}

#[test]
fn ids_are_read_from_each_list_shape() {
    assert_eq!(
        parse_ids(&json!({"data": [{"id": "b"}, {"id": "A"}, {"id": "b"}]})),
        Some(vec!["A".to_string(), "b".to_string()])
    );
    assert_eq!(
        parse_ids(&json!({"data": [{"name": "llama3"}]})),
        Some(vec!["llama3".to_string()])
    );
    assert_eq!(
        parse_ids(&json!(["y", "x"])),
        Some(vec!["x".to_string(), "y".to_string()])
    );
    assert_eq!(parse_ids(&json!({"data": []})), Some(vec![]));
    assert_eq!(parse_ids(&json!({})), None);
    assert_eq!(parse_ids(&json!({"data": "none"})), None);
}

#[tokio::test]
async fn a_list_of_ids_is_listed_without_duplicates_and_sorted_without_regard_to_case() {
    let body = r#"{"data":[{"id":"b"},{"id":"A"},{"id":"b"}]}"#;
    assert_eq!(
        list_with(Reply::Json(200, body.into()), None).await,
        listed(&["A", "b"])
    );
}

#[tokio::test]
async fn names_and_a_bare_array_are_listed() {
    let names = r#"{"data":[{"name":"llama3"}]}"#;
    assert_eq!(
        list_with(Reply::Json(200, names.into()), None).await,
        listed(&["llama3"])
    );
    assert_eq!(
        list_with(Reply::Json(200, r#"["x","y"]"#.into()), None).await,
        listed(&["x", "y"])
    );
}

#[tokio::test]
async fn an_empty_list_is_listed_with_no_ids() {
    assert_eq!(
        list_with(Reply::Json(200, r#"{"data":[]}"#.into()), None).await,
        listed(&[])
    );
}

#[tokio::test]
async fn a_body_with_no_list_is_not_listed() {
    assert_eq!(
        list_with(Reply::Json(200, "{}".into()), None).await,
        ModelList::NotListed
    );
    assert_eq!(
        list_with(Reply::Json(200, "<html>hello</html>".into()), None).await,
        ModelList::NotListed
    );
}

#[tokio::test]
async fn a_404_is_not_listed() {
    assert_eq!(
        list_with(Reply::Json(404, "{}".into()), Some("sk")).await,
        ModelList::NotListed
    );
}

#[tokio::test]
async fn a_401_is_key_rejected_with_a_key_and_key_needed_without() {
    assert_eq!(
        list_with(Reply::Json(401, "{}".into()), Some("sk")).await,
        ModelList::KeyRejected
    );
    assert_eq!(
        list_with(Reply::Json(401, "{}".into()), None).await,
        ModelList::KeyNeeded
    );
}

#[tokio::test]
async fn a_403_is_key_cannot_list_with_a_key_and_key_needed_without() {
    assert_eq!(
        list_with(Reply::Json(403, "{}".into()), Some("sk")).await,
        ModelList::KeyCannotList
    );
    assert_eq!(
        list_with(Reply::Json(403, "{}".into()), None).await,
        ModelList::KeyNeeded
    );
}

#[tokio::test]
async fn a_redirect_names_the_target_host_and_is_not_followed() {
    let mock = serve(HashMap::from([(
        "/v1/models",
        Reply::Redirect(302, "https://other.example/v1/models".into()),
    )]))
    .await;
    let outcome = list_models(&Client::new(endpoint(&mock.base, Some("sk"))).unwrap()).await;
    assert_eq!(
        outcome,
        ModelList::Redirected {
            host: Some("other.example".into())
        }
    );
    assert_eq!(mock.seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_relative_redirect_names_the_host_of_the_request() {
    let mock = serve(HashMap::from([(
        "/v1/models",
        Reply::Redirect(301, "/v2/models".into()),
    )]))
    .await;
    let outcome = list_models(&Client::new(endpoint(&mock.base, None)).unwrap()).await;
    assert_eq!(
        outcome,
        ModelList::Redirected {
            host: Some("127.0.0.1".into())
        }
    );
}

#[tokio::test]
async fn a_redirect_with_no_location_names_no_host() {
    // A `With` reply sends no `Location` header.
    let reply = Reply::With(Arc::new(|_| (302, String::new())));
    assert_eq!(
        list_with(reply, None).await,
        ModelList::Redirected { host: None }
    );
}

#[tokio::test]
async fn a_blank_id_is_dropped() {
    let body = r#"{"data":[{"id":""},{"id":"   "},{"id":"a"}]}"#;
    assert_eq!(
        list_with(Reply::Json(200, body.into()), None).await,
        listed(&["a"])
    );
    assert_eq!(parse_ids(&json!([" ", "b"])), Some(vec!["b".to_string()]));
}

#[tokio::test]
async fn other_error_statuses_are_server_errors_with_the_status() {
    assert_eq!(
        list_with(Reply::Json(500, "oops".into()), Some("sk")).await,
        ModelList::ServerError { status: Some(500) }
    );
    assert_eq!(
        list_with(Reply::Json(429, "slow down".into()), Some("sk")).await,
        ModelList::ServerError { status: Some(429) }
    );
}

#[tokio::test]
async fn a_body_over_the_byte_cap_is_a_server_error() {
    let mock = serve(HashMap::from([(
        "/v1/models",
        Reply::Json(
            200,
            format!(r#"{{"data":[{{"id":"{}"}}]}}"#, "m".repeat(512)),
        ),
    )]))
    .await;
    let small = Limits {
        max_bytes: 64,
        ..Limits::probe()
    };
    let client =
        Client::with_limits(endpoint(&mock.base, None), Limits::completion(), small).unwrap();
    assert_eq!(
        list_models(&client).await,
        ModelList::ServerError { status: None }
    );
}

#[tokio::test]
async fn a_server_that_never_answers_is_unreachable_within_the_deadline() {
    let mock = serve(HashMap::from([("/v1/models", Reply::Hang)])).await;
    let quick = Limits {
        connect: Duration::from_secs(1),
        response: Duration::from_millis(300),
        idle: Duration::from_millis(300),
        total: Duration::from_secs(1),
        max_bytes: 1 << 20,
    };
    let client = Client::with_limits(endpoint(&mock.base, None), quick, quick).unwrap();
    let started = Instant::now();
    assert_eq!(list_models(&client).await, ModelList::Unreachable);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn a_closed_port_is_unreachable() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let client = Client::new(endpoint(&format!("http://{addr}/v1"), None)).unwrap();
    assert_eq!(list_models(&client).await, ModelList::Unreachable);
}

#[tokio::test]
async fn the_request_is_a_get_of_models_with_the_key_only_as_a_bearer() {
    let mock = serve(HashMap::from([(
        "/v1/models",
        Reply::Json(200, r#"{"data":[]}"#.into()),
    )]))
    .await;
    list_models(&Client::new(endpoint(&format!("{}/", mock.base), Some("sk-test"))).unwrap()).await;
    list_models(&Client::new(endpoint(&mock.base, None)).unwrap()).await;
    let seen = mock.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].path, "/v1/models");
    assert_eq!(seen[0].authorization.as_deref(), Some("Bearer sk-test"));
    assert!(seen[0].body.is_empty());
    assert_eq!(seen[1].authorization, None);
}

#[test]
fn the_outcome_serializes_with_a_kind_tag() {
    assert_eq!(
        serde_json::to_value(listed(&["a"])).unwrap(),
        json!({"kind": "listed", "ids": ["a"]})
    );
    assert_eq!(
        serde_json::to_value(ModelList::KeyCannotList).unwrap(),
        json!({"kind": "key-cannot-list"})
    );
    assert_eq!(
        serde_json::to_value(ModelList::Redirected { host: None }).unwrap(),
        json!({"kind": "redirected", "host": null})
    );
}

/// The mock's address without `/v1`, as a user can type it.
fn root(mock: &common::Mock) -> String {
    mock.base.strip_suffix("/v1").unwrap().to_string()
}

fn paths(mock: &common::Mock) -> Vec<String> {
    mock.seen
        .lock()
        .unwrap()
        .iter()
        .map(|seen| format!("{} {}", seen.method, seen.path))
        .collect()
}

async fn check_at(address: &str, credential: Option<&str>) -> Checked {
    check(&Client::new(endpoint(address, credential)).unwrap()).await
}

const LIST: &str = r#"{"data":[{"id":"a"}]}"#;

#[tokio::test]
async fn covers_ae3_an_address_that_answers_only_under_v1_resolves_to_the_v1_address() {
    let mock = serve(HashMap::from([(
        "/v1/models",
        Reply::Json(200, LIST.into()),
    )]))
    .await;
    let checked = check_at(&root(&mock), None).await;
    assert_eq!(
        checked,
        Checked {
            list: listed(&["a"]),
            address: Some(format!("{}/v1", root(&mock))),
        }
    );
    assert_eq!(paths(&mock), ["GET /models", "GET /v1/models"]);
}

#[tokio::test]
async fn covers_ae4_an_address_that_ends_in_api_v1_and_answers_is_used_as_typed() {
    let mock = serve(HashMap::from([(
        "/api/v1/models",
        Reply::Json(200, LIST.into()),
    )]))
    .await;
    let typed = format!("{}/api/v1", root(&mock));
    let checked = check_at(&typed, Some("sk")).await;
    assert_eq!(
        checked,
        Checked {
            list: listed(&["a"]),
            address: Some(typed),
        }
    );
    assert_eq!(paths(&mock), ["GET /api/v1/models"]);
}

#[tokio::test]
async fn a_redirect_at_the_typed_address_does_not_start_the_v1_retry() {
    // The `/v1` address has a list, so a retry would succeed.
    let mock = serve(HashMap::from([
        (
            "/models",
            Reply::Redirect(302, "https://other.example/models".into()),
        ),
        ("/v1/models", Reply::Json(200, LIST.into())),
    ]))
    .await;
    let checked = check_at(&root(&mock), None).await;
    assert_eq!(
        checked,
        Checked {
            list: ModelList::Redirected {
                host: Some("other.example".into())
            },
            address: None,
        }
    );
    assert_eq!(paths(&mock), ["GET /models"]);
}

#[tokio::test]
async fn a_key_refusal_at_the_typed_address_does_not_start_the_v1_retry() {
    let mock = serve(HashMap::from([
        ("/models", Reply::Json(401, "{}".into())),
        ("/v1/models", Reply::Json(200, LIST.into())),
    ]))
    .await;
    let checked = check_at(&root(&mock), Some("sk")).await;
    assert_eq!(
        checked,
        Checked {
            list: ModelList::KeyRejected,
            address: None,
        }
    );
    assert_eq!(paths(&mock), ["GET /models"]);
}

#[tokio::test]
async fn a_server_with_no_list_and_a_chat_route_only_under_v1_resolves_to_the_v1_address() {
    let mock = serve(HashMap::from([(
        "/v1/chat/completions",
        Reply::Json(200, common::chat_reply("OK", "length")),
    )]))
    .await;
    let checked = check_at(&root(&mock), None).await;
    assert_eq!(
        checked,
        Checked {
            list: ModelList::ChatOnly,
            address: Some(format!("{}/v1", root(&mock))),
        }
    );
    assert_eq!(
        paths(&mock),
        [
            "GET /models",
            "GET /v1/models",
            "POST /chat/completions",
            "POST /v1/chat/completions"
        ]
    );
    let seen = mock.seen.lock().unwrap();
    assert_eq!(seen[3].json()["max_tokens"], 1);
}

#[tokio::test]
async fn a_server_with_no_list_and_a_chat_route_at_the_typed_address_keeps_the_typed_address() {
    let mock = serve(HashMap::from([(
        "/chat/completions",
        Reply::Json(200, common::chat_reply("OK", "stop")),
    )]))
    .await;
    let checked = check_at(&root(&mock), None).await;
    assert_eq!(
        checked,
        Checked {
            list: ModelList::ChatOnly,
            address: Some(root(&mock)),
        }
    );
    assert_eq!(
        paths(&mock),
        ["GET /models", "GET /v1/models", "POST /chat/completions"]
    );
}

/// Checks the address of a server that has no model list and answers the
/// chat route at the typed address with `reply`. The server also has a
/// chat route under `/v1`, so a retry would succeed. The check must stop
/// after the first chat request.
async fn check_chat_reply(reply: Reply, credential: Option<&str>) -> Checked {
    let mock = serve(HashMap::from([
        ("/chat/completions", reply),
        (
            "/v1/chat/completions",
            Reply::Json(200, common::chat_reply("OK", "stop")),
        ),
    ]))
    .await;
    let checked = check_at(&root(&mock), credential).await;
    assert_eq!(
        paths(&mock),
        ["GET /models", "GET /v1/models", "POST /chat/completions"]
    );
    checked
}

#[tokio::test]
async fn a_chat_route_that_refuses_with_no_key_needs_a_key_and_starts_no_v1_retry() {
    assert_eq!(
        check_chat_reply(Reply::Json(401, "{}".into()), None).await,
        Checked {
            list: ModelList::KeyNeeded,
            address: None,
        }
    );
}

#[tokio::test]
async fn a_chat_route_that_refuses_the_key_rejects_it_and_starts_no_v1_retry() {
    assert_eq!(
        check_chat_reply(Reply::Json(401, "{}".into()), Some("sk")).await,
        Checked {
            list: ModelList::KeyRejected,
            address: None,
        }
    );
}

#[tokio::test]
async fn a_chat_route_that_redirects_is_redirected_and_starts_no_v1_retry() {
    assert_eq!(
        check_chat_reply(
            Reply::Redirect(302, "https://other.example/chat/completions".into()),
            None
        )
        .await,
        Checked {
            list: ModelList::Redirected {
                host: Some("other.example".into())
            },
            address: None,
        }
    );
}

#[tokio::test]
async fn anthropic_lists_its_models_through_rig() {
    let mock = serve(HashMap::from([(
        "/v1/models",
        Reply::Json(
            200,
            r#"{"data":[{"id":"claude-b","display_name":"B"},{"id":"claude-a","display_name":"A"}],"has_more":false}"#.into(),
        ),
    )]))
    .await;
    let client = Client::new(Endpoint {
        provider: Provider::Anthropic,
        base_url: root(&mock),
        model: String::new(),
        credential: Some("ak".into()),
    })
    .unwrap();
    assert_eq!(
        list_models(&client).await,
        listed(&["claude-a", "claude-b"])
    );
    let seen = mock.seen.lock().unwrap();
    assert_eq!(seen[0].header("x-api-key"), Some("ak"));
}

const GEMINI_MODELS: &str = "/v1beta/models?pageSize=1000&key=gk";

fn gemini(base: &str) -> Client {
    Client::new(Endpoint {
        provider: Provider::Gemini,
        base_url: base.into(),
        model: "gemini-2.5-flash".into(),
        credential: Some("gk".into()),
    })
    .unwrap()
}

#[tokio::test]
async fn gemini_lists_its_models_through_rig_without_the_models_prefix() {
    let mock = serve(HashMap::from([(
        GEMINI_MODELS,
        Reply::Json(
            200,
            r#"{"models":[{"name":"models/gemini-2.5-flash"},{"name":"models/gemini-2.5-pro"}]}"#
                .into(),
        ),
    )]))
    .await;
    assert_eq!(
        list_models(&gemini(&root(&mock))).await,
        listed(&["gemini-2.5-flash", "gemini-2.5-pro"])
    );
}

#[tokio::test]
async fn a_gemini_key_that_is_not_valid_is_key_rejected() {
    let mock = serve(HashMap::from([(
        GEMINI_MODELS,
        Reply::Json(
            400,
            r#"{"error":{"code":400,"message":"API key not valid. Please pass a valid API key."}}"#
                .into(),
        ),
    )]))
    .await;
    assert_eq!(
        list_models(&gemini(&root(&mock))).await,
        ModelList::KeyRejected
    );
}

#[tokio::test]
async fn ollama_lists_its_models_through_rig() {
    let mock = serve(HashMap::from([(
        "/api/tags",
        Reply::Json(
            200,
            r#"{"models":[{"name":"qwen3:8b","model":"qwen3:8b"},{"name":"llama3:8b","model":"llama3:8b"}]}"#.into(),
        ),
    )]))
    .await;
    let client = Client::new(Endpoint {
        provider: Provider::Ollama,
        base_url: root(&mock),
        model: String::new(),
        credential: None,
    })
    .unwrap();
    assert_eq!(
        list_models(&client).await,
        listed(&["llama3:8b", "qwen3:8b"])
    );
}
