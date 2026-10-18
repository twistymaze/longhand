//! The transport that Rig sends through: redirects, deadlines, the size
//! cap and errors with no URL (R3, KTD1).

mod common;

use std::collections::HashMap;
use std::time::Duration;

use common::{Reply, serve};
use rig_core::http_client::{self, HttpClientExt, Request};
use summarize::{Limits, Transport, TransportError};

/// Send a POST to `url` and read the whole body.
async fn post(transport: &Transport, url: &str) -> Result<(u16, Vec<u8>), http_client::Error> {
    let request = Request::post(url).body(b"{}".to_vec()).unwrap();
    let response = transport.send::<_, Vec<u8>>(request).await?;
    let status = response.status().as_u16();
    let body = response.into_body().await?;
    Ok((status, body))
}

/// The transport error in `error`.
fn kind(error: &http_client::Error) -> &TransportError {
    TransportError::find(error).unwrap_or_else(|| panic!("not a transport error: {error:?}"))
}

/// The caller's total deadline in the deadline tests. It is longer than
/// the short deadline of each test and shorter than its long deadline.
const TOTAL: Duration = Duration::from_secs(10);

fn limits(response: Duration, idle: Duration) -> Limits {
    Limits {
        response,
        idle,
        ..Limits::completion()
    }
}

/// A server that answers each connection with plain HTTP. An https request
/// to it fails the TLS handshake.
async fn plain_server_for_https() -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 8192];
            let _ = socket.read(&mut buf).await;
            let _ = socket.write_all(b"HTTP/1.1 400 X\r\n\r\n").await;
        }
    });
    format!("https://{address}")
}

#[tokio::test]
async fn a_redirect_is_an_error_and_nothing_reaches_its_target() {
    let target = serve(HashMap::from([(
        "/v1/chat/completions",
        Reply::Json(200, "{}".into()),
    )]))
    .await;
    let location = format!("{}/chat/completions", target.base);
    let mock = serve(HashMap::from([(
        "/v1/chat/completions",
        Reply::Redirect(302, location.clone()),
    )]))
    .await;
    let transport = Transport::new().unwrap();
    let err = post(&transport, &format!("{}/chat/completions", mock.base))
        .await
        .unwrap_err();
    let TransportError::Redirect {
        status,
        location: named,
    } = kind(&err)
    else {
        panic!("{err:?}");
    };
    assert_eq!(*status, 302);
    assert_eq!(named, &location);
    assert_eq!(mock.seen.lock().unwrap().len(), 1);
    assert!(target.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_body_larger_than_8_mib_is_too_large_for_success_and_error_replies() {
    let big = "a".repeat((8 << 20) + 1);
    let fits = "a".repeat(8 << 20);
    let mock = serve(HashMap::from([
        ("/ok", Reply::Json(200, big.clone())),
        ("/error", Reply::Json(500, big)),
        ("/fits", Reply::Json(200, fits)),
    ]))
    .await;
    let root = mock.base.trim_end_matches("/v1");
    let transport = Transport::new().unwrap();
    for path in ["/ok", "/error"] {
        let err = post(&transport, &format!("{root}{path}"))
            .await
            .unwrap_err();
        assert!(
            matches!(kind(&err), TransportError::TooLarge(n) if *n == 8 << 20),
            "{path}: {err:?}"
        );
    }
    let (status, body) = post(&transport, &format!("{root}/fits")).await.unwrap();
    assert_eq!((status, body.len()), (200, 8 << 20));
}

#[tokio::test]
async fn an_error_reply_gives_its_status_and_body() {
    // Rig reads the status and the body of an error reply to classify it.
    let mock = serve(HashMap::from([(
        "/v1/chat/completions",
        Reply::Json(401, r#"{"error":"bad key"}"#.into()),
    )]))
    .await;
    let transport = Transport::new().unwrap();
    let (status, body) = post(&transport, &format!("{}/chat/completions", mock.base))
        .await
        .unwrap();
    assert_eq!(status, 401);
    assert_eq!(body, br#"{"error":"bad key"}"#);
}

#[tokio::test]
async fn a_server_that_stops_mid_body_fails_at_the_idle_deadline() {
    let mock = serve(HashMap::from([(
        "/v1/chat/completions",
        Reply::Stall(200, 100, "{\"cho".into()),
    )]))
    .await;
    // The response deadline and the caller's total deadline are long. Only
    // the idle deadline can end the request.
    let transport =
        Transport::with_limits(limits(Duration::from_secs(60), Duration::from_millis(200)))
            .unwrap();
    let result = tokio::time::timeout(
        TOTAL,
        post(&transport, &format!("{}/chat/completions", mock.base)),
    )
    .await
    .expect("the idle deadline ends the request before the total deadline");
    let err = result.unwrap_err();
    assert!(
        matches!(kind(&err), TransportError::IdleDeadline),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_server_that_sends_no_headers_fails_at_the_response_deadline() {
    // The idle deadline and the caller's total deadline are long. Only the
    // response deadline can end the request.
    let mock = serve(HashMap::from([("/v1/chat/completions", Reply::Hang)])).await;
    let transport =
        Transport::with_limits(limits(Duration::from_millis(200), Duration::from_secs(60)))
            .unwrap();
    let result = tokio::time::timeout(
        TOTAL,
        post(&transport, &format!("{}/chat/completions", mock.base)),
    )
    .await
    .expect("the response deadline ends the request before the total deadline");
    let err = result.unwrap_err();
    assert!(
        matches!(kind(&err), TransportError::ResponseDeadline),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_closed_port_is_unreachable() {
    let transport = Transport::new().unwrap();
    let err = post(
        &transport,
        &format!("{}/chat/completions", common::closed_base()),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(kind(&err), TransportError::Unreachable(_)),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_tls_failure_is_tls_and_not_unreachable() {
    let transport = Transport::new().unwrap();
    let base = plain_server_for_https().await;
    let err = post(&transport, &format!("{base}/v1/chat/completions"))
        .await
        .unwrap_err();
    assert!(matches!(kind(&err), TransportError::Tls(_)), "{err:?}");
}

#[tokio::test]
async fn a_transport_error_has_no_url_path_and_no_query() {
    let transport = Transport::new().unwrap();
    let closed = common::closed_base();
    let tls = plain_server_for_https().await;
    for base in [closed.as_str(), tls.as_str()] {
        let url = format!("{base}/secret-path/chat/completions?key=secret");
        let err = post(&transport, &url).await.unwrap_err();
        assert!(
            matches!(
                kind(&err),
                TransportError::Unreachable(_) | TransportError::Tls(_)
            ),
            "{err:?}"
        );
        for text in [err.to_string(), format!("{err:?}")] {
            assert!(!text.contains("key="), "{text}");
            assert!(!text.contains("secret"), "{text}");
        }
    }
}

#[tokio::test]
async fn streamed_and_multipart_requests_are_refused() {
    let transport = Transport::new().unwrap();
    let url = format!("{}/chat/completions", common::closed_base());
    let request = Request::post(&url).body(b"{}".to_vec()).unwrap();
    let Err(err) = transport.send_streaming(request).await else {
        panic!("a streamed request was sent");
    };
    assert!(
        matches!(kind(&err), TransportError::Unsupported(_)),
        "{err:?}"
    );
    let form = http_client::MultipartForm::new();
    let request = Request::post(&url).body(form).unwrap();
    let Err(err) = transport.send_multipart::<Vec<u8>>(request).await else {
        panic!("a multipart request was sent");
    };
    assert!(
        matches!(kind(&err), TransportError::Unsupported(_)),
        "{err:?}"
    );
}
