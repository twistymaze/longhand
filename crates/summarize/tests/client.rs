//! The chat-completions client: redirects, deadlines, credentials, errors.

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{Reply, completion, serve};
use summarize::client::Message;
use summarize::{Client, Endpoint, Limits, SummarizeError};

fn endpoint(base: &str, credential: Option<&str>) -> Endpoint {
    Endpoint {
        base_url: base.into(),
        model: "m".into(),
        credential: credential.map(str::to_string),
    }
}

fn hello() -> Vec<Message> {
    vec![Message {
        role: "user",
        content: "hello".into(),
    }]
}

#[tokio::test]
async fn a_redirect_is_a_typed_failure_and_nothing_reaches_its_target() {
    let target = serve(HashMap::from([(
        "/v1/chat/completions",
        Reply::Json(200, completion("stolen")),
    )]))
    .await;
    let mock = serve(HashMap::from([(
        "/v1/chat/completions",
        Reply::Redirect(307, format!("{}/chat/completions", target.base)),
    )]))
    .await;
    let client = Client::new(endpoint(&mock.base, Some("secret"))).unwrap();
    let err = client.complete(&hello(), 100).await.unwrap_err();
    let SummarizeError::Redirect { status, location } = &err else {
        panic!("{err:?}");
    };
    assert_eq!(*status, 307);
    assert!(location.starts_with(&target.base));
    assert!(target.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_request_waits_for_the_endpoint_to_come_back_after_a_network_failure() {
    // A Mac that wakes from sleep can have no network for some seconds. The
    // request tries again after each wait, and the endpoint answers later.
    let port = common::free_port();
    let client = Client::new(endpoint(&format!("http://127.0.0.1:{port}/v1"), None))
        .unwrap()
        .with_retry_waits(vec![Duration::from_millis(50); 40]);
    tokio::spawn(async move {
        // Until then, each connection is refused.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .unwrap();
        // The server task owns the listener and keeps it open.
        common::serve_on(
            listener,
            HashMap::from([("/v1/chat/completions", Reply::Json(200, completion("back")))]),
        );
    });
    let reply = client.complete(&hello(), 100).await.unwrap();
    assert_eq!(reply.text, "back");
}

#[tokio::test]
async fn a_connection_that_closes_before_the_answer_is_not_sent_again() {
    // The endpoint read the request, so the model can be at work on it.
    use tokio::io::AsyncReadExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let connections = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = Arc::clone(&connections);
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut buf = [0u8; 8192];
            let _ = socket.read(&mut buf).await;
            drop(socket);
        }
    });
    let client = Client::new(endpoint(&base, None))
        .unwrap()
        .with_retry_waits(vec![Duration::from_millis(10); 3]);
    let err = client.complete(&hello(), 100).await.unwrap_err();
    assert!(matches!(err, SummarizeError::Transport(_)), "{err:?}");
    assert_eq!(connections.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_request_with_no_retry_waits_fails_at_once_without_the_network() {
    let client = Client::new(endpoint(&common::closed_base(), None)).unwrap();
    let err = client.complete(&hello(), 100).await.unwrap_err();
    assert!(matches!(err, SummarizeError::Transport(_)), "{err:?}");
}

#[tokio::test]
async fn a_reply_that_breaks_after_it_starts_is_not_sent_again() {
    // The endpoint received the prompt and began to answer. A new try would
    // make the model write the reply again.
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let connections = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = Arc::clone(&connections);
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut buf = [0u8; 8192];
            let _ = socket.read(&mut buf).await;
            let _ = socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{\"cho")
                .await;
            drop(socket);
        }
    });
    let client = Client::new(endpoint(&base, None))
        .unwrap()
        .with_retry_waits(vec![Duration::from_millis(10); 3]);
    let err = client.complete(&hello(), 100).await.unwrap_err();
    assert!(matches!(err, SummarizeError::Transport(_)), "{err:?}");
    assert_eq!(connections.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_request_that_reaches_the_endpoint_is_not_sent_again() {
    // Only a failure to reach the endpoint is tried again. An answer, also
    // an error status, is the result.
    let mock = serve(HashMap::from([(
        "/v1/chat/completions",
        Reply::Json(500, r#"{"error":"down"}"#.into()),
    )]))
    .await;
    let client = Client::new(endpoint(&mock.base, None))
        .unwrap()
        .with_retry_waits(vec![Duration::from_millis(10); 3]);
    let err = client.complete(&hello(), 100).await.unwrap_err();
    assert!(
        matches!(err, SummarizeError::Http { status: 500, .. }),
        "{err:?}"
    );
    assert_eq!(mock.seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn an_endpoint_that_never_answers_hits_the_deadline() {
    let mock = serve(HashMap::from([("/v1/chat/completions", Reply::Hang)])).await;
    let quick = Limits {
        connect: Duration::from_secs(1),
        response: Duration::from_millis(300),
        idle: Duration::from_millis(300),
        total: Duration::from_secs(1),
        max_bytes: 1 << 20,
    };
    let client = Client::with_limits(endpoint(&mock.base, None), quick, quick).unwrap();
    let started = Instant::now();
    let err = client.complete(&hello(), 100).await.unwrap_err();
    assert!(
        matches!(err, SummarizeError::Deadline("response")),
        "{err:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn the_credential_goes_only_as_a_bearer_to_the_configured_endpoint() {
    let mock = serve(HashMap::from([(
        "/v1/chat/completions",
        Reply::Json(200, completion("fine")),
    )]))
    .await;
    let with = Client::new(endpoint(&mock.base, Some("sk-test"))).unwrap();
    assert_eq!(with.complete(&hello(), 100).await.unwrap().text, "fine");
    let without = Client::new(endpoint(&mock.base, None)).unwrap();
    without.complete(&hello(), 100).await.unwrap();
    let seen = mock.seen.lock().unwrap();
    assert_eq!(seen[0].authorization.as_deref(), Some("Bearer sk-test"));
    assert_eq!(seen[1].authorization, None);
    assert!(seen[0].body.contains("\"max_tokens\":100"));
}

#[tokio::test]
async fn a_context_rejection_is_pressure_and_a_cut_off_reply_is_not() {
    let mock = serve(HashMap::from([(
        "/v1/chat/completions",
        Reply::Json(
            400,
            r#"{"error":{"message":"This model's maximum context length is 8192 tokens"}}"#.into(),
        ),
    )]))
    .await;
    let err = Client::new(endpoint(&mock.base, None))
        .unwrap()
        .complete(&hello(), 100)
        .await
        .unwrap_err();
    assert!(err.is_context_pressure(), "{err:?}");

    let cut = serve(HashMap::from([(
        "/v1/chat/completions",
        Reply::Json(
            200,
            r#"{"choices":[{"message":{"content":"half"},"finish_reason":"length"}]}"#.into(),
        ),
    )]))
    .await;
    let err = Client::new(endpoint(&cut.base, None))
        .unwrap()
        .complete(&hello(), 100)
        .await
        .unwrap_err();
    assert!(matches!(err, SummarizeError::ReplyCutOff));
    assert!(
        !err.is_context_pressure(),
        "the prompt fit; the reply needs room"
    );
}

#[tokio::test]
async fn a_model_refusing_max_tokens_is_asked_again_with_max_completion_tokens() {
    let reply = Reply::With(Arc::new(|body: &str| {
        if body.contains("\"max_tokens\"") {
            (
                400,
                r#"{"error":{"message":"Unsupported parameter: 'max_tokens'. Use 'max_completion_tokens' instead."}}"#.into(),
            )
        } else {
            (200, completion("ok"))
        }
    }));
    let mock = serve(HashMap::from([("/v1/chat/completions", reply)])).await;
    let text = Client::new(endpoint(&mock.base, None))
        .unwrap()
        .complete(&hello(), 50)
        .await
        .unwrap()
        .text;
    assert_eq!(text, "ok");
    assert_eq!(mock.seen.lock().unwrap().len(), 2);
}

#[test]
fn plain_http_to_another_machine_is_flagged_and_loopback_is_not() {
    let flagged = |url: &str| {
        Client::new(endpoint(url, None))
            .unwrap()
            .crosses_network_unencrypted()
    };
    assert!(flagged("http://192.168.1.20:1234/v1"));
    assert!(flagged("http://gpu-box.local/v1"));
    assert!(!flagged("http://localhost:1234/v1"));
    assert!(!flagged("http://127.0.0.1:8080/v1"));
    assert!(!flagged("https://api.example.com/v1"));
    assert!(matches!(
        Client::new(endpoint("ftp://x/v1", None)),
        Err(SummarizeError::BadUrl(_))
    ));
}
