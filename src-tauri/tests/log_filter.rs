//! The app's log filter on summary requests (R3, KTD4): Rig writes nothing
//! to the log, and the request lines name no prompt, reply, key or URL path.
//!
//! The test logger gives each record to the fern logger that
//! tauri-plugin-log builds, with the app's levels. It also keeps each record
//! before the filter, to prove that Rig wrote records that the filter
//! removed.

#[path = "../../crates/summarize/tests/common/mod.rs"]
mod common;

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use common::{Reply, anthropic_reply, chat_reply, gemini_reply, ollama_reply, serve};
use longhand_lib::{LOG_LEVEL, LOG_TARGET_LEVELS};
use summarize::client::{Label, Message, RequestKind};
use summarize::{Client, Endpoint, Provider};

const KEY: &str = "k-secret-4c1";
const SYSTEM_PROMPT: &str = "system-prompt-4c1";
const USER_PROMPT: &str = "user-prompt-4c1";
const REPLY_TEXT: &str = "reply-text-4c1";
const ERROR_BODY: &str = "error-body-4c1";

/// One log record: its target and its text.
type Line = (String, String);

/// Keeps every record, then gives it to the app's filter and keeps the
/// records that the filter writes.
struct Capture {
    filter: Box<dyn log::Log>,
    before_filter: Mutex<Vec<Line>>,
    written: &'static Mutex<Vec<Line>>,
}

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        self.before_filter
            .lock()
            .unwrap()
            .push((record.target().to_string(), record.args().to_string()));
        self.filter.log(record);
    }

    fn flush(&self) {}
}

fn capture() -> &'static Capture {
    static CAPTURE: OnceLock<&'static Capture> = OnceLock::new();
    CAPTURE.get_or_init(|| {
        let written: &'static Mutex<Vec<Line>> = Box::leak(Box::new(Mutex::new(Vec::new())));
        let dispatch = LOG_TARGET_LEVELS
            .iter()
            .fold(fern::Dispatch::new().level(LOG_LEVEL), |d, &(t, l)| {
                d.level_for(t, l)
            })
            .chain(fern::Output::call(move |record| {
                written
                    .lock()
                    .unwrap()
                    .push((record.target().to_string(), record.args().to_string()));
            }));
        let (_, filter) = dispatch.into_log();
        let capture: &'static Capture = Box::leak(Box::new(Capture {
            filter,
            before_filter: Mutex::new(Vec::new()),
            written,
        }));
        log::set_logger(capture).expect("no other logger in this test binary");
        // Every level reaches `Capture`, so `before_filter` holds Rig's
        // records at each level.
        log::set_max_level(log::LevelFilter::Trace);
        capture
    })
}

fn written() -> Vec<Line> {
    capture().written.lock().unwrap().clone()
}

fn before_filter() -> Vec<Line> {
    capture().before_filter.lock().unwrap().clone()
}

fn is_rig_target(target: &str) -> bool {
    ["rig", "rig_core"]
        .iter()
        .any(|name| target == *name || target.starts_with(&format!("{name}::")))
}

/// The path that `provider` sends a summary request to, without a query.
fn path_for(provider: Provider) -> &'static str {
    match provider {
        Provider::Anthropic => "/v1/messages",
        Provider::Gemini => "/v1beta/models/m:generateContent",
        Provider::Ollama => "/api/chat",
        _ => "/v1/chat/completions",
    }
}

/// The path with the query that the mock server records for `provider`.
fn route_for(provider: Provider) -> &'static str {
    match provider {
        Provider::Gemini => "/v1beta/models/m:generateContent?key=k-secret-4c1",
        other => path_for(other),
    }
}

fn reply_for(provider: Provider) -> String {
    match provider {
        Provider::Anthropic => anthropic_reply(REPLY_TEXT, "end_turn"),
        Provider::Gemini => gemini_reply(REPLY_TEXT, "STOP"),
        Provider::Ollama => ollama_reply(REPLY_TEXT, "stop"),
        _ => chat_reply(REPLY_TEXT, "stop"),
    }
}

/// A client for `provider` that sends to the mock server at `mock_base`.
fn client(provider: Provider, mock_base: &str) -> Client {
    let base_url = match provider {
        // Gemini's paths start at the server root.
        Provider::Gemini => mock_base.trim_end_matches("/v1").to_string(),
        _ => mock_base.to_string(),
    };
    Client::new(Endpoint {
        provider,
        base_url,
        model: "m".into(),
        credential: Some(KEY.into()),
    })
    .unwrap()
}

fn messages() -> Vec<Message> {
    vec![
        Message {
            role: "system",
            content: SYSTEM_PROMPT.into(),
        },
        Message {
            role: "user",
            content: USER_PROMPT.into(),
        },
    ]
}

/// The text in `line` that the log must not contain.
fn forbidden_text(line: &str, also: &[&str]) -> Option<String> {
    let mut forbidden = vec![KEY, SYSTEM_PROMPT, USER_PROMPT, REPLY_TEXT, "?key="];
    forbidden.extend(Provider::ALL.map(path_for));
    forbidden.extend(also);
    forbidden
        .into_iter()
        .find(|text| line.contains(text))
        .map(str::to_string)
}

#[tokio::test]
async fn a_summary_request_writes_no_rig_line_and_no_request_content() {
    capture();
    for provider in Provider::ALL {
        let mock = serve(HashMap::from([(
            route_for(provider),
            Reply::Json(200, reply_for(provider)),
        )]))
        .await;
        let note = format!("log-ok-{provider:?}");
        let reply = client(provider, &mock.base)
            .complete(
                &Label::note(&note, RequestKind::Single),
                &messages(),
                100,
                Some(4096),
            )
            .await
            .unwrap_or_else(|e| panic!("{provider:?}: {e}"));
        assert_eq!(reply.text, REPLY_TEXT, "{provider:?}");
    }

    // Rig wrote records for these requests, so the filter has work to do.
    assert!(
        before_filter()
            .iter()
            .any(|(target, _)| is_rig_target(target)),
        "Rig wrote no log record; the test cannot show the filter"
    );
    let written = written();
    for (target, text) in &written {
        assert!(
            !is_rig_target(target),
            "a Rig line was written: {target} {text}"
        );
        if let Some(found) = forbidden_text(text, &[]) {
            panic!("a line holds {found:?}: {target} {text}");
        }
    }
    for provider in Provider::ALL {
        let note = format!("note log-ok-{provider:?},");
        assert!(
            written
                .iter()
                .any(|(_, text)| text.starts_with("request end: ")
                    && text.contains(&note)
                    && text.ends_with(", HTTP 2xx")),
            "{provider:?}: no end line"
        );
    }
}

#[tokio::test]
async fn a_failed_summary_request_writes_the_error_kind_and_status_only() {
    capture();
    let provider = Provider::Custom;
    let mock = serve(HashMap::from([(
        path_for(provider),
        Reply::Json(500, format!(r#"{{"error":{{"message":"{ERROR_BODY}"}}}}"#)),
    )]))
    .await;
    let note = "log-failed";
    client(provider, &mock.base)
        .complete(
            &Label::note(note, RequestKind::Single),
            &messages(),
            100,
            None,
        )
        .await
        .expect_err("the server answered HTTP 500");

    let written = written();
    for (target, text) in &written {
        assert!(
            !is_rig_target(target),
            "a Rig line was written: {target} {text}"
        );
        if let Some(found) = forbidden_text(text, &[ERROR_BODY]) {
            panic!("a line holds {found:?}: {target} {text}");
        }
    }
    let end_lines: Vec<&String> = written
        .iter()
        .map(|(_, text)| text)
        .filter(|text| text.starts_with("request end: ") && text.contains("note log-failed,"))
        .collect();
    assert_eq!(end_lines.len(), 1, "{end_lines:?}");
    assert!(end_lines[0].ends_with(" ms, HTTP 500"), "{}", end_lines[0]);
}
