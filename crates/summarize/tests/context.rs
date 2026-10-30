//! The context-budget probe chain against scripted servers.

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use common::{Reply, serve};
use summarize::client::{Label, RequestKind};
use summarize::context::{self, DEFAULT_LIMIT};
use summarize::{Budget, BudgetSource, Client, Endpoint, Limits, Provider};

fn probe_label() -> Label {
    Label::note("s1", RequestKind::Probe)
}

fn client(base: &str, model: &str) -> Client {
    Client::new(Endpoint {
        provider: Provider::Custom,
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
    let budget = context::probe(&client(&mock.base, "qwen3-8b"), &probe_label())
        .await
        .unwrap();
    assert_eq!(
        budget,
        Some(Budget {
            limit: 8_192,
            source: BudgetSource::LmStudio,
            output_max: None,
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
    let budget = context::probe(&client(&mock.base, "claude"), &probe_label())
        .await
        .unwrap();
    assert_eq!(
        budget,
        Some(Budget {
            limit: 200_000,
            source: BudgetSource::Litellm,
            output_max: None,
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
    let budget = context::probe(&client(&mock.base, "local"), &probe_label())
        .await
        .unwrap();
    assert_eq!(
        budget,
        Some(Budget {
            limit: 32_768,
            source: BudgetSource::Vllm,
            output_max: None,
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
        context::probe(&client(&llama.base, "any"), &probe_label())
            .await
            .unwrap(),
        Some(Budget {
            limit: 16_384,
            source: BudgetSource::LlamaCpp,
            output_max: None,
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
        context::probe(&client(&router.base, "x/y"), &probe_label())
            .await
            .unwrap(),
        Some(Budget {
            limit: 1_000_000,
            source: BudgetSource::Openrouter,
            output_max: None,
        })
    );
}

#[tokio::test]
async fn a_server_answering_no_probe_resolves_to_the_default() {
    let mock = serve(HashMap::new()).await;
    let probed = context::probe(&client(&mock.base, "m"), &probe_label())
        .await
        .unwrap();
    assert_eq!(probed, None);
    assert_eq!(
        context::resolve(None, probed),
        Budget {
            limit: DEFAULT_LIMIT,
            source: BudgetSource::Default,
            output_max: None,
        }
    );
}

#[tokio::test]
async fn an_endpoint_that_cannot_be_reached_is_an_error_not_the_default() {
    // A Mac that wakes from sleep for a few seconds can have no network.
    // That is not an endpoint with no context data, so the caller must not
    // use the default budget for the run.
    let probed = context::probe(&client(&common::closed_base(), "m"), &probe_label()).await;
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
        let probed = context::probe(&client(&mock.base, "m"), &probe_label()).await;
        assert!(
            matches!(probed, Err(summarize::SummarizeError::Http { status: s, .. }) if s == status),
            "HTTP {status}: {probed:?}"
        );
    }
}

#[tokio::test]
async fn a_first_probe_that_hits_its_deadline_is_an_error_not_the_default() {
    // A slow network or a busy server gives no answer in time. That tells
    // nothing about the context, so the run must not keep the default.
    let mock = serve(HashMap::from([("/v1/model/info", Reply::Hang)])).await;
    let quick = Limits {
        connect: Duration::from_secs(1),
        response: Duration::from_millis(300),
        idle: Duration::from_millis(300),
        total: Duration::from_secs(1),
        max_bytes: 1 << 20,
    };
    let client = Client::with_limits(
        Endpoint {
            provider: Provider::Custom,
            base_url: mock.base.clone(),
            model: "m".into(),
            credential: None,
        },
        Limits::completion(),
        quick,
    )
    .unwrap();
    let probed = context::probe(&client, &probe_label()).await;
    assert!(
        matches!(probed, Err(summarize::SummarizeError::Deadline("response"))),
        "{probed:?}"
    );
}

#[tokio::test]
async fn a_first_probe_refused_as_unauthorized_goes_on_to_the_next_probe() {
    // A key can be limited to the completion routes, so 401 or 403 on the
    // first probe does not prove a bad key. The chain goes on.
    let vllm = r#"{"data":[{"id":"m","max_model_len":32768}]}"#;
    for status in [401, 403] {
        let mock = serve(HashMap::from([
            ("/v1/model/info", Reply::Json(status, "{}".into())),
            ("/v1/models", Reply::Json(200, vllm.into())),
        ]))
        .await;
        let probed = context::probe(&client(&mock.base, "m"), &probe_label())
            .await
            .unwrap();
        assert_eq!(
            probed,
            Some(Budget {
                limit: 32_768,
                source: BudgetSource::Vllm,
                output_max: None,
            }),
            "HTTP {status}"
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
    let probed = context::probe(&client(&mock.base, "m"), &probe_label())
        .await
        .unwrap();
    assert_eq!(
        probed,
        Some(Budget {
            limit: 32_768,
            source: BudgetSource::Vllm,
            output_max: None,
        })
    );
    // 501 says that the server does not have the route.
    let mock = serve(HashMap::from([(
        "/v1/model/info",
        Reply::Json(501, "{}".into()),
    )]))
    .await;
    assert_eq!(
        context::probe(&client(&mock.base, "m"), &probe_label())
            .await
            .unwrap(),
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
    let probed = context::probe(&client(&base, "m"), &probe_label())
        .await
        .unwrap();
    assert_eq!(probed, None);
}

#[test]
fn the_user_setting_wins_over_the_probe() {
    let lm = Budget {
        limit: 8_192,
        source: BudgetSource::LmStudio,
        output_max: None,
    };
    let probed = Some(lm);
    assert_eq!(
        context::resolve(Some(32_000), probed),
        Budget {
            limit: 32_000,
            source: BudgetSource::User,
            output_max: None,
        }
    );
    assert_eq!(context::resolve(None, probed), lm);
}

fn provider_client(provider: Provider, base: &str, model: &str, key: Option<&str>) -> Client {
    Client::new(Endpoint {
        provider,
        base_url: base.into(),
        model: model.into(),
        credential: key.map(str::to_string),
    })
    .unwrap()
}

#[tokio::test]
async fn an_ollama_model_reports_its_context_length_from_api_show() {
    let mock = serve(HashMap::from([(
        "/api/show",
        Reply::With(Arc::new(|body: &str| {
            let request: serde_json::Value = serde_json::from_str(body).unwrap();
            if request["model"] != "qwen3:8b" {
                return (404, r#"{"error":"model not found"}"#.into());
            }
            (
                200,
                r#"{"model_info":{"general.architecture":"qwen3","qwen3.context_length":32768,"qwen3.embedding_length":4096}}"#.into(),
            )
        })),
    )]))
    .await;
    let root = mock.base.strip_suffix("/v1").unwrap();
    let client = provider_client(Provider::Ollama, root, "qwen3:8b", None);
    assert_eq!(
        context::probe(&client, &probe_label()).await.unwrap(),
        Some(Budget {
            limit: 32_768,
            source: BudgetSource::Ollama,
            output_max: None,
        })
    );
    let seen = mock.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
}

#[test]
fn the_ollama_context_length_is_read_without_the_architecture_name() {
    let body = serde_json::json!({"model_info": {"llama.context_length": 8192}});
    assert_eq!(context::parse_ollama_show(&body), Some(8_192));
    assert_eq!(context::parse_ollama_show(&serde_json::json!({})), None);
}

#[tokio::test]
async fn a_gemini_model_reports_the_input_token_limit_of_its_model_list_entry() {
    let mock = serve(HashMap::from([(
        "/v1beta/models?pageSize=1000&key=gk",
        Reply::Json(
            200,
            r#"{"models":[
                {"name":"models/gemini-2.5-pro","inputTokenLimit":2000000,"outputTokenLimit":65536},
                {"name":"models/gemini-2.5-flash","inputTokenLimit":1048576,"outputTokenLimit":65536}
            ]}"#
            .into(),
        ),
    )]))
    .await;
    let root = mock.base.strip_suffix("/v1").unwrap();
    let client = provider_client(Provider::Gemini, root, "gemini-2.5-flash", Some("gk"));
    assert_eq!(
        context::probe(&client, &probe_label()).await.unwrap(),
        Some(Budget {
            limit: 1_048_576,
            source: BudgetSource::Gemini,
            output_max: None,
        })
    );
}

#[tokio::test]
async fn anthropic_and_openai_give_no_probed_limit_so_the_entered_value_or_the_default_applies() {
    for provider in [Provider::Anthropic, Provider::OpenAi] {
        // Each route answers with a limit, so a probe request would find one.
        let mock = serve(HashMap::from([
            (
                "/v1/models",
                Reply::Json(200, r#"{"data":[{"id":"m","max_model_len":4096}]}"#.into()),
            ),
            (
                "/v1/model/info",
                Reply::Json(
                    200,
                    r#"{"data":[{"model_name":"m","model_info":{"max_input_tokens":4096}}]}"#
                        .into(),
                ),
            ),
        ]))
        .await;
        let client = provider_client(provider, &mock.base, "m", Some("k"));
        let probed = context::probe(&client, &probe_label()).await.unwrap();
        assert_eq!(probed, None, "{provider:?}");
        assert!(mock.seen.lock().unwrap().is_empty(), "{provider:?}");
        assert_eq!(
            context::resolve(None, probed),
            Budget {
                limit: DEFAULT_LIMIT,
                source: BudgetSource::Default,
                output_max: None,
            }
        );
        assert_eq!(
            context::resolve(Some(50_000), probed),
            Budget {
                limit: 50_000,
                source: BudgetSource::User,
                output_max: None,
            }
        );
    }
}

#[tokio::test]
async fn litellm_reports_the_output_maximum_from_max_output_tokens() {
    let mock = serve(HashMap::from([(
        "/v1/model/info",
        Reply::Json(
            200,
            r#"{"data":[{"model_name":"m","model_info":{"max_input_tokens":131072,"max_tokens":8000,"max_output_tokens":8000}}]}"#
                .into(),
        ),
    )]))
    .await;
    let budget = context::probe(&client(&mock.base, "m"), &probe_label())
        .await
        .unwrap();
    assert_eq!(
        budget,
        Some(Budget {
            limit: 131_072,
            source: BudgetSource::Litellm,
            output_max: Some(8_000),
        })
    );
}

/// KTD3: LiteLLM can fill the legacy `max_tokens` with the input limit, so
/// only `max_output_tokens` gives the output maximum.
#[tokio::test]
async fn litellm_max_tokens_alone_gives_no_output_maximum() {
    let mock = serve(HashMap::from([(
        "/v1/model/info",
        Reply::Json(
            200,
            r#"{"data":[{"model_name":"m","model_info":{"max_input_tokens":131072,"max_tokens":131072}}]}"#
                .into(),
        ),
    )]))
    .await;
    let budget = context::probe(&client(&mock.base, "m"), &probe_label())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(budget.output_max, None);
}

/// An older saved plan has a budget with no output maximum.
#[test]
fn a_budget_without_an_output_maximum_still_reads() {
    let budget: Budget = serde_json::from_str(r#"{"limit":32000,"source":"litellm"}"#).unwrap();
    assert_eq!(budget.output_max, None);
    assert_eq!(budget.limit, 32_000);
}

/// KTD3: Rig's catalog gives the output maximum of known hosted models.
#[test]
fn the_catalog_gives_the_output_maximum_of_a_known_hosted_model() {
    assert_eq!(
        context::catalog_output_max(Provider::Anthropic, "claude-haiku-4-5"),
        Some(64_000)
    );
    assert_eq!(
        context::catalog_output_max(Provider::Anthropic, "claude-unknown-9"),
        None
    );
    assert_eq!(
        context::catalog_output_max(Provider::Gemini, "models/gemini-2.5-flash"),
        Some(65_536)
    );
    // Local servers are not in the catalog.
    assert_eq!(
        context::catalog_output_max(Provider::LmStudio, "claude-haiku-4-5"),
        None
    );
}

// ------------------------------------------------------- reply allowance (U4)

use summarize::tokens::{Allowance, ReplyKind, reply_allowance, split_reserve, with_margin};

/// Covers AE1: LM Studio gets 90% of the context at the first request.
#[test]
fn lm_studio_gets_the_whole_room_at_the_first_request() {
    let kind = ReplyKind::of(Provider::LmStudio, BudgetSource::LmStudio);
    let allowance = reply_allowance(131_072, with_margin(6_571), None, kind);
    assert_eq!(
        allowance,
        Allowance {
            first: 117_964,
            ceiling: 117_964
        }
    );
    // The split reserve does not change (R3).
    assert_eq!(split_reserve(131_072), 8_192);
}

/// Covers AE2, R2: a hosted model with no known output maximum.
#[test]
fn an_unknown_hosted_model_gets_the_safe_value() {
    for provider in [Provider::Anthropic, Provider::OpenAi, Provider::Gemini] {
        let kind = ReplyKind::of(provider, BudgetSource::Default);
        assert_eq!(
            reply_allowance(200_000, 5_000, None, kind),
            Allowance {
                first: 16_384,
                ceiling: 16_384
            },
            "{provider:?}"
        );
    }
}

/// KTD2: a known output maximum of 64,000 starts at 16,384. Anthropic
/// never goes above 21,333; OpenAI can go up to the output maximum.
#[test]
fn a_known_output_maximum_starts_at_the_safe_value() {
    let anthropic = ReplyKind::of(Provider::Anthropic, BudgetSource::Default);
    assert_eq!(
        reply_allowance(200_000, 5_000, Some(64_000), anthropic),
        Allowance {
            first: 16_384,
            ceiling: 21_333
        }
    );
    let openai = ReplyKind::of(Provider::OpenAi, BudgetSource::Default);
    assert_eq!(
        reply_allowance(200_000, 5_000, Some(64_000), openai),
        Allowance {
            first: 16_384,
            ceiling: 64_000
        }
    );
}

#[test]
fn ollama_starts_at_32_768() {
    let kind = ReplyKind::of(Provider::Ollama, BudgetSource::User);
    assert_eq!(
        reply_allowance(131_072, 2_000, None, kind),
        Allowance {
            first: 32_768,
            ceiling: 117_964
        }
    );
}

/// KTD2: Custom gets the whole room only when a llama.cpp or vLLM probe
/// gave its limit.
#[test]
fn custom_gets_the_whole_room_only_after_a_llama_cpp_or_vllm_probe() {
    for source in [BudgetSource::Vllm, BudgetSource::LlamaCpp] {
        let kind = ReplyKind::of(Provider::Custom, source);
        assert_eq!(reply_allowance(131_072, 2_000, None, kind).first, 117_964);
    }
    let litellm = ReplyKind::of(Provider::Custom, BudgetSource::Litellm);
    assert_eq!(reply_allowance(131_072, 2_000, None, litellm).first, 16_384);
    // A LiteLLM output maximum of 8,000 caps the allowance.
    assert_eq!(
        reply_allowance(131_072, 2_000, Some(8_000), litellm),
        Allowance {
            first: 8_000,
            ceiling: 8_000
        }
    );
}

/// R1: the allowance is never more than the context that the prompt
/// leaves.
#[test]
fn the_allowance_is_never_more_than_the_room_left() {
    let full = ReplyKind::of(Provider::LmStudio, BudgetSource::LmStudio);
    assert_eq!(reply_allowance(8_192, 6_000, None, full).first, 2_192);
    let hosted = ReplyKind::of(Provider::OpenAi, BudgetSource::Default);
    assert_eq!(reply_allowance(8_192, 6_000, None, hosted).first, 2_192);
}
