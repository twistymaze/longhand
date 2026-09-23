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
    let budget = context::probe(&client(&mock.base, "qwen3-8b")).await;
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
    let budget = context::probe(&client(&mock.base, "claude")).await;
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
    let budget = context::probe(&client(&mock.base, "local")).await;
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
        context::probe(&client(&llama.base, "any")).await,
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
        context::probe(&client(&router.base, "x/y")).await,
        Some(Budget {
            limit: 1_000_000,
            source: BudgetSource::Openrouter
        })
    );
}

#[tokio::test]
async fn a_server_answering_no_probe_resolves_to_the_default() {
    let mock = serve(HashMap::new()).await;
    let probed = context::probe(&client(&mock.base, "m")).await;
    assert_eq!(probed, None);
    assert_eq!(
        context::resolve(None, probed, None),
        Budget {
            limit: DEFAULT_LIMIT,
            source: BudgetSource::Default
        }
    );
}

#[test]
fn the_user_setting_wins_and_a_learned_limit_caps_it() {
    let lm = Budget {
        limit: 8_192,
        source: BudgetSource::LmStudio,
    };
    let probed = Some(lm);
    assert_eq!(
        context::resolve(Some(32_000), probed, None),
        Budget {
            limit: 32_000,
            source: BudgetSource::User
        }
    );
    assert_eq!(
        context::resolve(Some(32_000), probed, Some(20_000)),
        Budget {
            limit: 20_000,
            source: BudgetSource::Learned
        }
    );
    // A learned limit above the resolved one says nothing new.
    assert_eq!(context::resolve(None, probed, Some(50_000)), lm);
}
