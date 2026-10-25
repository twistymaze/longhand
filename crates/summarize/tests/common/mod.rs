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
    /// Wait for the time, then answer with the status and the JSON body.
    Later(std::time::Duration, u16, String),
    /// Read the request, wait until the test opens the gate, then answer
    /// with the reply.
    Gated(Gate, Box<Reply>),
    /// Send the headers with the status and a `Content-Length` of the
    /// given value, then the start of the body, then keep the connection
    /// open and send no more.
    Stall(u16, usize, String),
}

/// A gate that holds the replies of [`Reply::Gated`] until the test opens
/// it. It does not block a thread. Once open, it stays open.
#[derive(Clone)]
pub struct Gate(Arc<tokio::sync::watch::Sender<bool>>);

impl Default for Gate {
    fn default() -> Self {
        Self(Arc::new(tokio::sync::watch::channel(false).0))
    }
}

impl Gate {
    pub fn open(&self) {
        self.0.send_replace(true);
    }

    async fn wait(&self) {
        let mut open = self.0.subscribe();
        let _ = open.wait_for(|open| *open).await;
    }
}

#[derive(Debug, Clone)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub authorization: Option<String>,
    /// Every header, with its name in lowercase.
    pub headers: Vec<(String, String)>,
    pub body: String,
}

pub struct Mock {
    pub base: String,
    pub seen: Arc<Mutex<Vec<Seen>>>,
    /// The number of requests in `seen`.
    recorded: Arc<tokio::sync::watch::Sender<usize>>,
}

impl Mock {
    /// Wait until the server has read `count` requests.
    pub async fn requests(&self, count: usize) {
        let mut recorded = self.recorded.subscribe();
        let _ = recorded.wait_for(|n| *n >= count).await;
    }
}

/// A local socket that is bound but does not listen, and its port. No
/// connection to the port succeeds until the test calls `listen` on the
/// socket. While the socket is open, no other test can bind the port.
#[allow(dead_code)]
pub fn closed_socket() -> (tokio::net::TcpSocket, u16) {
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.bind(([127, 0, 0, 1], 0).into()).unwrap();
    let port = socket.local_addr().unwrap().port();
    (socket, port)
}

/// A base URL that no connection can reach. A connection to port 0 fails at
/// once, and no test server can listen on port 0.
#[allow(dead_code)]
pub fn closed_base() -> String {
    "http://127.0.0.1:0/v1".to_string()
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
    let recorded = Arc::new(tokio::sync::watch::channel(0).0);
    let count = Arc::clone(&recorded);
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
            let count = Arc::clone(&count);
            tokio::spawn(async move {
                let Some(request) = read_request(&mut socket).await else {
                    return;
                };
                let total = {
                    let mut log = log.lock().unwrap();
                    log.push(request.clone());
                    log.len()
                };
                count.send_replace(total);
                let mut reply = routes
                    .get(&request.path)
                    .cloned()
                    .unwrap_or(Reply::Json(404, "{}".into()));
                while let Reply::Gated(gate, inner) = reply {
                    gate.wait().await;
                    reply = *inner;
                }
                let (status, headers, body) = match reply {
                    Reply::Gated(..) => unreachable!(),
                    Reply::Hang => {
                        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                        return;
                    }
                    Reply::Stall(status, length, start) => {
                        let head = format!(
                            "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {length}\r\n\r\n{start}"
                        );
                        let _ = socket.write_all(head.as_bytes()).await;
                        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                        return;
                    }
                    Reply::Json(status, body) => (status, String::new(), body),
                    Reply::Later(wait, status, body) => {
                        tokio::time::sleep(wait).await;
                        (status, String::new(), body)
                    }
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
        recorded,
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
    let mut headers = Vec::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
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
        headers,
        body: String::from_utf8_lossy(&buf[header_end..]).to_string(),
    })
}

impl Seen {
    /// The value of the header `name` (lowercase), if the request had it.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// The body as JSON.
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).unwrap()
    }
}

/// An OpenAI Chat Completions reply with `text` and `finish_reason`.
pub fn chat_reply(text: &str, finish_reason: &str) -> String {
    serde_json::json!({
        "choices": [{"message": {"role": "assistant", "content": text}, "finish_reason": finish_reason}]
    })
    .to_string()
}

/// An Anthropic Messages reply with `text` and `stop_reason`.
pub fn anthropic_reply(text: &str, stop_reason: &str) -> String {
    serde_json::json!({
        "id": "msg_1",
        "type": "message",
        "role": "assistant",
        "model": "m",
        "content": [{"type": "text", "text": text}],
        "stop_reason": stop_reason,
        "usage": {"input_tokens": 1, "output_tokens": 1}
    })
    .to_string()
}

/// A Gemini `generateContent` reply with `text` and `finishReason`.
pub fn gemini_reply(text: &str, finish_reason: &str) -> String {
    serde_json::json!({
        "candidates": [{
            "content": {"parts": [{"text": text}], "role": "model"},
            "finishReason": finish_reason
        }]
    })
    .to_string()
}

/// An Ollama `/api/chat` reply with `text` and `done_reason`.
pub fn ollama_reply(text: &str, done_reason: &str) -> String {
    serde_json::json!({
        "model": "m",
        "message": {"role": "assistant", "content": text},
        "done": true,
        "done_reason": done_reason
    })
    .to_string()
}

pub fn completion(text: &str) -> String {
    serde_json::json!({
        "choices": [{"message": {"role": "assistant", "content": text}, "finish_reason": "stop"}]
    })
    .to_string()
}
