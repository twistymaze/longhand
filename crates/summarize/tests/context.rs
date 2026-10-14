//! The context-budget probe chain against scripted servers.

mod common;

use std::collections::HashMap;

use common::{Reply, serve};
use summarize::context::{self, DEFAULT_LIMIT};
use summarize::{Budget, BudgetSource, Client, Endpoint};

fn client(base: &str, model: &str) -> Client {
    Client::new(Endpoint {
        base_url: base.into(),
        model: model.into(),
        credential: None,
    })
    .unwrap()
}

#[tokio::test]
async fn lm_studio_reports_the_loaded_context() {
    let mock = serve(HashMap::from([(
        "/api/v0/models",
        Reply::Json(
            200,
            r#"{"data":[{"id":"qwen3-8b","loaded_context_length":8192,"max_context_length":32768}]}"#.into(),
        ),
    )]))
    .await;
    let budget = context::probe(&client(&mock.base, "qwen3-8b"))
        .await
        .unwrap();
    assert_eq!(
        budget,
        Some(Budget {
            limit: 8_192,
            source: BudgetSource::LmStudio
        })
    );
}

#[tokio::test]
async fn litellm_reports_the_input_limit_not_the_output_cap() {
    let mock = serve(HashMap::from([(
        "/v1/model/info",
        Reply::Json(
            200,
            r#"{"data":[
                {"model_name":"other","model_info":{"max_input_tokens":1000}},
                {"model_name":"claude","model_info":{"max_input_tokens":200000,"max_tokens":64000}}
            ]}"#
            .into(),
        ),
    )]))
    .await;
    let budget = context::probe(&client(&mock.base, "claude")).await.unwrap();
    assert_eq!(
        budget,
        Some(Budget {
            limit: 200_000,
            source: BudgetSource::Litellm
        })
    );
}

#[tokio::test]
async fn a_row_without_context_metadata_falls_through_to_the_next_probe() {
    let mock = serve(HashMap::from([
        (
            "/v1/model/info",
            Reply::Json(
                200,
                r#"{"data":[{"model_name":"local","model_info":{}}]}"#.into(),
            ),
        ),
        (
            "/v1/models",
            Reply::Json(
                200,
                r#"{"data":[{"id":"local","max_model_len":32768}]}"#.into(),
            ),
        ),
    ]))
    .await;
    let budget = context::probe(&client(&mock.base, "local")).await.unwrap();
    assert_eq!(
        budget,
        Some(Budget {
            limit: 32_768,
            source: BudgetSource::Vllm
        })
    );
}

#[tokio::test]
async fn llama_cpp_and_openrouter_answer_in_their_own_shapes() {
    let llama = serve(HashMap::from([(
        "/props",
        Reply::Json(
            200,
            r#"{"default_generation_settings":{"n_ctx":16384}}"#.into(),
        ),
    )]))
    .await;
    assert_eq!(
        context::probe(&client(&llama.base, "any")).await.unwrap(),
        Some(Budget {
            limit: 16_384,
            source: BudgetSource::LlamaCpp
        })
    );
    let router = serve(HashMap::from([(
        "/v1/models",
        Reply::Json(
            200,
            r#"{"data":[{"id":"x/y","context_length":1000000}]}"#.into(),
        ),
    )]))
    .await;
    assert_eq!(
        context::probe(&client(&router.base, "x/y")).await.unwrap(),
        Some(Budget {
            limit: 1_000_000,
            source: BudgetSource::Openrouter
        })
    );
}

#[tokio::test]
async fn a_server_answering_no_probe_resolves_to_the_default() {
    let mock = serve(HashMap::new()).await;
    let probed = context::probe(&client(&mock.base, "m")).await.unwrap();
    assert_eq!(probed, None);
    assert_eq!(
        context::resolve(None, probed),
        Budget {
            limit: DEFAULT_LIMIT,
            source: BudgetSource::Default
        }
    );
}

#[tokio::test]
async fn an_endpoint_that_cannot_be_reached_is_an_error_not_the_default() {
    // A Mac that wakes from sleep for a few seconds can have no network.
    // That is not an endpoint with no context data, so the caller must not
    // use the default budget for the run.
    let probed = context::probe(&client(&common::closed_base(), "m")).await;
    assert!(
        matches!(probed, Err(summarize::SummarizeError::Transport(_))),
        "{probed:?}"
    );
}

#[tokio::test]
async fn a_server_error_from_a_probe_is_an_error_not_the_default() {
    // A proxy that restarts answers 502 for a few seconds. That tells
    // nothing about the context, so the run must not use the small default
    // budget and keep it with the note.
    for status in [500, 502, 503, 504, 408, 429] {
        let mock = serve(HashMap::from([(
            "/v1/model/info",
            Reply::Json(status, "{}".into()),
        )]))
        .await;
        let probed = context::probe(&client(&mock.base, "m")).await;
        assert!(
            matches!(probed, Err(summarize::SummarizeError::Http { status: s, .. }) if s == status),
            "HTTP {status}: {probed:?}"
        );
    }
}

#[tokio::test]
async fn a_server_error_from_a_later_probe_or_a_route_not_implemented_is_a_miss() {
    // A gateway can answer 503 for a route that it does not have, so only
    // the first probe stops on a server error. The chain goes on to the
    // route that names the context.
    let vllm = r#"{"data":[{"id":"m","max_model_len":32768}]}"#;
    let mock = serve(HashMap::from([
        ("/api/v0/models", Reply::Json(503, "{}".into())),
        ("/v1/models", Reply::Json(200, vllm.into())),
    ]))
    .await;
    let probed = context::probe(&client(&mock.base, "m")).await.unwrap();
    assert_eq!(
        probed,
        Some(Budget {
            limit: 32_768,
            source: BudgetSource::Vllm
        })
    );
    // 501 says that the server does not have the route.
    let mock = serve(HashMap::from([(
        "/v1/model/info",
        Reply::Json(501, "{}".into()),
    )]))
    .await;
    assert_eq!(
        context::probe(&client(&mock.base, "m")).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn a_later_probe_that_fails_after_an_answer_is_a_miss() {
    // The first probe got an answer, so the endpoint is reachable. A later
    // probe that the server drops tells nothing more about the context.
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let mut first = true;
        while let Ok((mut socket, _)) = listener.accept().await {
            if first {
                first = false;
                let mut buf = [0u8; 8192];
                let _ = socket.read(&mut buf).await;
                let _ = socket
                    .write_all(
                        b"HTTP/1.1 404 X\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                    )
                    .await;
            }
            drop(socket);
        }
    });
    let probed = context::probe(&client(&base, "m")).await.unwrap();
    assert_eq!(probed, None);
}

#[test]
fn the_user_setting_wins_over_the_probe() {
    let lm = Budget {
        limit: 8_192,
        source: BudgetSource::LmStudio,
    };
    let probed = Some(lm);
    assert_eq!(
        context::resolve(Some(32_000), probed),
        Budget {
            limit: 32_000,
            source: BudgetSource::User
        }
    );
    assert_eq!(context::resolve(None, probed), lm);
}
