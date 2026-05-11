use crate::audit::AuditEvent;
use crate::config::{Account, Config};
use crate::registry::{AgentConn, ForwardRequest};
use crate::{auth, AppState};
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};
use tokio::sync::mpsc;

const MAX_BODY: usize = 32 * 1024 * 1024;

enum Backend<'a> {
    Direct { upstream: &'a str, api_key: &'a str },
    Agent { name: String, conn: Arc<AgentConn> },
}

impl Backend<'_> {
    fn audit_name(&self) -> String {
        match self {
            Backend::Direct { .. } => "anthropic-api-key".into(),
            Backend::Agent { name, .. } => format!("agent:{}", name),
        }
    }
}

fn pick_backend<'a>(
    state: &'a AppState,
    config: &'a Config,
    account: &Account,
) -> Option<Backend<'a>> {
    for name in &account.allowed_agents {
        if config.agents.iter().any(|a| &a.name == name) {
            if let Some(conn) = state.registry.get(name) {
                return Some(Backend::Agent {
                    name: name.clone(),
                    conn,
                });
            }
        }
    }
    if account
        .allowed_providers
        .iter()
        .any(|p| p == "anthropic" || p == "*")
    {
        if let Some(an) = &config.anthropic {
            return Some(Backend::Direct {
                upstream: &an.upstream,
                api_key: &an.api_key,
            });
        }
    }
    None
}

pub async fn anthropic_messages(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    req: Request<Body>,
) -> Response {
    let account = match auth::authenticate(&state.config.accounts, &headers) {
        Ok(a) => a,
        Err(reason) => {
            state.audit.write(AuditEvent {
                provider: Some("anthropic".into()),
                status: Some(401),
                reason: Some(reason.into()),
                ..AuditEvent::new("auth_denied")
            });
            return (StatusCode::UNAUTHORIZED, reason).into_response();
        }
    };

    let Some(backend) = pick_backend(&state, &state.config, account) else {
        state.audit.write(AuditEvent {
            account: Some(account.name.clone()),
            provider: Some("anthropic".into()),
            status: Some(403),
            reason: Some("no allowed backend".into()),
            ..AuditEvent::new("auth_denied")
        });
        return (StatusCode::FORBIDDEN, "no allowed backend").into_response();
    };

    let body_bytes = match axum::body::to_bytes(req.into_body(), MAX_BODY).await {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("body: {}", e)).into_response(),
    };

    let parsed: Option<serde_json::Value> = serde_json::from_slice(&body_bytes).ok();
    let model = parsed
        .as_ref()
        .and_then(|v| v.get("model").and_then(|m| m.as_str()).map(String::from));
    let stream = parsed
        .as_ref()
        .and_then(|v| v.get("stream").and_then(|s| s.as_bool()))
        .unwrap_or(false);

    let backend_name = backend.audit_name();
    let account_name = account.name.clone();

    let result = match backend {
        Backend::Direct { upstream, api_key } => {
            forward_direct(&state, upstream, api_key, &headers, body_bytes.clone()).await
        }
        Backend::Agent { conn, .. } => forward_agent(conn, &headers, body_bytes.clone()).await,
    };

    match result {
        Ok((status, resp_headers, body)) => {
            state.audit.write(AuditEvent {
                account: Some(account_name),
                provider: Some("anthropic".into()),
                backend: Some(backend_name),
                model,
                status: Some(status.as_u16()),
                stream: Some(stream),
                ..AuditEvent::new("messages_request")
            });
            let mut builder = Response::builder().status(status);
            for k in ["content-type", "anthropic-request-id"] {
                if let Some(v) = resp_headers.get(k) {
                    builder = builder.header(k, v);
                }
            }
            builder.body(body).unwrap_or_else(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("resp build: {}", e),
                )
                    .into_response()
            })
        }
        Err((status, reason)) => {
            state.audit.write(AuditEvent {
                account: Some(account_name),
                provider: Some("anthropic".into()),
                backend: Some(backend_name),
                model,
                status: Some(status.as_u16()),
                stream: Some(stream),
                reason: Some(reason.clone()),
                ..AuditEvent::new("messages_request")
            });
            (status, reason).into_response()
        }
    }
}

async fn forward_direct(
    state: &AppState,
    upstream: &str,
    api_key: &str,
    headers: &HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, HeaderMap, Body), (StatusCode, String)> {
    let url = format!("{}/v1/messages", upstream.trim_end_matches('/'));
    let mut builder = state
        .http
        .post(&url)
        .header("content-type", "application/json")
        .header("x-api-key", api_key);
    if headers.get("anthropic-version").is_none() {
        builder = builder.header("anthropic-version", "2023-06-01");
    }
    for k in ["anthropic-version", "anthropic-beta"] {
        if let Some(v) = headers.get(k) {
            builder = builder.header(k, v);
        }
    }
    let resp = builder
        .body(body.to_vec())
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("upstream: {}", e)))?;
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let resp_headers = resp.headers().clone();
    let body_stream = resp.bytes_stream();
    Ok((status, resp_headers, Body::from_stream(body_stream)))
}

async fn forward_agent(
    conn: Arc<AgentConn>,
    headers: &HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, HeaderMap, Body), (StatusCode, String)> {
    let fwd_headers = collect_forward_headers(headers);
    let req = ForwardRequest {
        method: "POST".into(),
        path: "/v1/messages".into(),
        headers: fwd_headers,
        body: body.to_vec(),
    };
    let inflight = conn
        .dispatch(req)
        .await
        .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "agent offline".into()))?;
    let crate::registry::InflightResponse { head, body, guard } = inflight;
    let head_ok = match head.await {
        Ok(Ok(h)) => h,
        Ok(Err(msg)) => return Err((StatusCode::BAD_GATEWAY, format!("agent: {}", msg))),
        Err(_) => return Err((StatusCode::SERVICE_UNAVAILABLE, "agent disconnected".into())),
    };
    let status = StatusCode::from_u16(head_ok.status).unwrap_or(StatusCode::BAD_GATEWAY);
    let resp_headers = headers_from_map(&head_ok.headers);
    let body_stream = ResponseStream {
        rx: body,
        _guard: guard,
    };
    Ok((status, resp_headers, Body::from_stream(body_stream)))
}

fn collect_forward_headers(headers: &HeaderMap) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for k in ["anthropic-version", "anthropic-beta", "content-type"] {
        if let Some(v) = headers.get(k).and_then(|v| v.to_str().ok()) {
            out.insert(k.into(), v.into());
        }
    }
    out
}

fn headers_from_map(map: &HashMap<String, String>) -> HeaderMap {
    let mut h = HeaderMap::new();
    for (k, v) in map {
        if let (Ok(name), Ok(val)) = (
            HeaderName::try_from(k.as_str()),
            HeaderValue::from_str(v.as_str()),
        ) {
            h.insert(name, val);
        }
    }
    h
}

struct ResponseStream {
    rx: mpsc::Receiver<Result<Bytes, String>>,
    _guard: crate::registry::InflightGuard,
}

impl Stream for ResponseStream {
    type Item = Result<Bytes, std::io::Error>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        match self.rx.poll_recv(cx) {
            Poll::Ready(Some(Ok(b))) => Poll::Ready(Some(Ok(b))),
            Poll::Ready(Some(Err(msg))) => Poll::Ready(Some(Err(std::io::Error::other(msg)))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}
