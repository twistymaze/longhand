//! The HTTP transport that Rig sends each request through (KTD1, R3).
//!
//! Rig's own transport uses a second TLS stack and has no size cap. This
//! transport uses the same reqwest client as [`crate::Client`]: it follows
//! no redirects, and it applies the connect deadline, the response
//! deadline until the headers, the idle deadline between body chunks and
//! the size cap. The cap applies to a success reply and to an error reply.
//!
//! The transport does not apply the total deadline. The caller puts each
//! Rig call under a `tokio` timeout.
//!
//! Each error is a [`TransportError`] in [`Error::Instance`]. Its text is
//! written by Longhand and has no URL. Use [`TransportError::find`] to get
//! it back from a Rig error.

use std::future::Future;

use bytes::Bytes;
use rig_core::http_client::{
    Error, HttpClientExt, LazyBody, MultipartForm, Request, Response, StreamingResponse,
};
use tokio::time::timeout;

use crate::client::{Limits, http_client, is_tls_failure, redirect_location, without_url};

/// Why a request through [`Transport`] failed.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// The endpoint answered with a 3xx status. The transport does not
    /// follow it.
    #[error(
        "the endpoint answered HTTP {status} redirecting to {location}; Longhand follows no redirects"
    )]
    Redirect { status: u16, location: String },
    /// The connection to the endpoint failed, so the endpoint did not
    /// receive the request. Only this kind can be sent again (R3).
    #[error("could not reach the endpoint: {0}")]
    Unreachable(String),
    /// The TLS handshake failed. A new try gets the same failure.
    #[error("the secure connection to the endpoint failed: {0}")]
    Tls(String),
    /// The connection failed after the request was possibly sent.
    #[error("the connection to the endpoint failed: {0}")]
    Connection(String),
    /// The request could not be made, for example from a URL that reqwest
    /// does not accept.
    #[error("the request could not be made: {0}")]
    Request(String),
    #[error("no response from the endpoint within its deadline")]
    ResponseDeadline,
    #[error("no data from the endpoint within its deadline")]
    IdleDeadline,
    #[error("the endpoint's reply exceeded {0} bytes")]
    TooLarge(usize),
    /// Longhand sends no streamed or multipart request.
    #[error("the transport does not send {0} requests")]
    Unsupported(&'static str),
}

impl TransportError {
    /// The transport error in `error`, if `error` came from [`Transport`].
    pub fn find(error: &Error) -> Option<&TransportError> {
        match error {
            Error::Instance(inner) => inner.downcast_ref(),
            _ => None,
        }
    }

    fn into_rig(self) -> Error {
        Error::instance(self)
    }
}

/// A Rig HTTP client over one reqwest client, under [`Limits`]. The
/// `total` field of the limits is not used here.
#[derive(Debug, Clone)]
pub struct Transport {
    http: reqwest::Client,
    limits: Limits,
}

impl Transport {
    /// A transport with the deadlines and the size cap of a summary request.
    pub fn new() -> Result<Self, TransportError> {
        Self::with_limits(Limits::completion())
    }

    pub fn with_limits(limits: Limits) -> Result<Self, TransportError> {
        let http =
            http_client(limits.connect).map_err(|e| TransportError::Request(without_url(e)))?;
        Ok(Self { http, limits })
    }

    async fn send_bytes(
        http: reqwest::Client,
        limits: Limits,
        request: Request<Bytes>,
    ) -> Result<Response<LazyBody<Bytes>>, TransportError> {
        let request = reqwest::Request::try_from(request)
            .map_err(|e| TransportError::Request(without_url(e)))?;
        let response = timeout(limits.response, http.execute(request))
            .await
            .map_err(|_| TransportError::ResponseDeadline)?
            .map_err(|e| {
                // Only a failed connection proves that the endpoint did not
                // receive the request.
                if is_tls_failure(&e) {
                    TransportError::Tls(without_url(e))
                } else if e.is_connect() {
                    TransportError::Unreachable(without_url(e))
                } else {
                    TransportError::Connection(without_url(e))
                }
            })?;
        let status = response.status();
        if status.is_redirection() {
            return Err(TransportError::Redirect {
                status: status.as_u16(),
                location: redirect_location(response.headers()),
            });
        }
        let mut builder = Response::builder()
            .status(status)
            .version(response.version());
        if let Some(headers) = builder.headers_mut() {
            headers.extend(response.headers().clone());
        }
        let body: LazyBody<Bytes> = Box::pin(async move {
            read_body(response, limits)
                .await
                .map_err(TransportError::into_rig)
        });
        builder
            .body(body)
            .map_err(|_| TransportError::Request("the reply headers are not valid".into()))
    }
}

/// Read the whole body under the idle deadline and the size cap.
async fn read_body(
    mut response: reqwest::Response,
    limits: Limits,
) -> Result<Bytes, TransportError> {
    let mut body = Vec::new();
    loop {
        match timeout(limits.idle, response.chunk()).await {
            Err(_) => return Err(TransportError::IdleDeadline),
            Ok(Err(e)) => return Err(TransportError::Connection(without_url(e))),
            Ok(Ok(None)) => return Ok(Bytes::from(body)),
            Ok(Ok(Some(bytes))) => {
                if body.len() + bytes.len() > limits.max_bytes {
                    return Err(TransportError::TooLarge(limits.max_bytes));
                }
                body.extend_from_slice(&bytes);
            }
        }
    }
}

impl HttpClientExt for Transport {
    fn send<T, U>(
        &self,
        req: Request<T>,
    ) -> impl Future<Output = Result<Response<LazyBody<U>>, Error>> + Send + 'static
    where
        T: Into<Bytes> + Send,
        U: From<Bytes> + Send + 'static,
    {
        let (parts, body) = req.into_parts();
        let request = Request::from_parts(parts, body.into());
        let (http, limits) = (self.http.clone(), self.limits);
        async move {
            let response = Self::send_bytes(http, limits, request)
                .await
                .map_err(TransportError::into_rig)?;
            Ok(response
                .map(|body| -> LazyBody<U> { Box::pin(async move { body.await.map(U::from) }) }))
        }
    }

    fn send_multipart<U>(
        &self,
        _req: Request<MultipartForm>,
    ) -> impl Future<Output = Result<Response<LazyBody<U>>, Error>> + Send + 'static
    where
        U: From<Bytes> + Send + 'static,
    {
        std::future::ready(Err(TransportError::Unsupported("multipart").into_rig()))
    }

    fn send_streaming<T>(
        &self,
        _req: Request<T>,
    ) -> impl Future<Output = Result<StreamingResponse, Error>> + Send
    where
        T: Into<Bytes> + Send,
    {
        std::future::ready(Err(TransportError::Unsupported("streamed").into_rig()))
    }
}
