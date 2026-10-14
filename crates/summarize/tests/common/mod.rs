//! A scripted HTTP/1.1 server on loopback, enough for one request per
//! connection, recording what it was sent.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Builds a reply from the request body.
pub type Responder = Arc<dyn Fn(&str) -> (u16, String) + Send + Sync>;

#[derive(Clone)]
pub enum Reply {
    Json(u16, String),
    Redirect(u16, String),
    /// Accept, read the request, never answer.
    Hang,
    /// Answer with a function of the request body.
    With(Responder),
}

#[derive(Debug, Clone)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub authorization: Option<String>,
    pub body: String,
}

pub struct Mock {
    pub base: String,
    pub seen: Arc<Mutex<Vec<Seen>>>,
}

/// A base URL on a local port that nothing listens on.
#[allow(dead_code)]
pub fn closed_base() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("http://127.0.0.1:{port}/v1")
}

pub async fn serve(routes: HashMap<&'static str, Reply>) -> Mock {
    serve_on(TcpListener::bind("127.0.0.1:0").await.unwrap(), routes)
}

/// Serve `routes` on `listener`, for a test that starts the server on a
/// port that it chose before.
#[allow(dead_code)]
pub fn serve_on(listener: TcpListener, routes: HashMap<&'static str, Reply>) -> Mock {
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    let routes: HashMap<String, Reply> = routes
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let routes = routes.clone();
            let log = Arc::clone(&log);
            tokio::spawn(async move {
                let Some(request) = read_request(&mut socket).await else {
                    return;
                };
                log.lock().unwrap().push(request.clone());
                let reply = routes
                    .get(&request.path)
                    .cloned()
                    .unwrap_or(Reply::Json(404, "{}".into()));
                let (status, headers, body) = match reply {
                    Reply::Hang => {
                        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                        return;
                    }
                    Reply::Json(status, body) => (status, String::new(), body),
                    Reply::Redirect(status, location) => {
                        (status, format!("Location: {location}\r\n"), String::new())
                    }
                    Reply::With(f) => {
                        let (status, body) = f(&request.body);
                        (status, String::new(), body)
                    }
                };
                let response = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    Mock {
        base: format!("http://{addr}/v1"),
        seen,
    }
}

async fn read_request(socket: &mut tokio::net::TcpStream) -> Option<Seen> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let header_end = loop {
        let n = socket.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head.lines();
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_string();
    let path = first.next()?.to_string();
    let mut length = 0;
    let mut authorization = None;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            match k.trim().to_ascii_lowercase().as_str() {
                "content-length" => length = v.trim().parse().unwrap_or(0),
                "authorization" => authorization = Some(v.trim().to_string()),
                _ => {}
            }
        }
    }
    while buf.len() < header_end + length {
        let n = socket.read(&mut chunk).await.ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    Some(Seen {
        method,
        path,
        authorization,
        body: String::from_utf8_lossy(&buf[header_end..]).to_string(),
    })
}

pub fn completion(text: &str) -> String {
    serde_json::json!({
        "choices": [{"message": {"role": "assistant", "content": text}, "finish_reason": "stop"}]
    })
    .to_string()
}
