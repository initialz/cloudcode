use crate::tunnel::ClientMsg;
use crate::AppState;
use base64::Engine;
use futures::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Forward a single hub-pushed request to the upstream Anthropic API and
/// stream the response back as `RespHead`/`RespChunk`/`RespEnd` frames.
pub async fn forward(
    state: Arc<AppState>,
    req_id: u64,
    _method: String,
    _path: String,
    headers: HashMap<String, String>,
    body_b64: String,
    send: mpsc::Sender<ClientMsg>,
) {
    let body = match base64::engine::general_purpose::STANDARD.decode(body_b64.as_bytes()) {
        Ok(b) => b,
        Err(e) => {
            let _ = send
                .send(ClientMsg::RespError {
                    req_id,
                    message: format!("body decode: {}", e),
                })
                .await;
            return;
        }
    };

    let creds = state.credentials.snapshot();
    let url = format!(
        "{}/v1/messages",
        state.config.claude.upstream.trim_end_matches('/')
    );

    let beta_value = state.config.claude.anthropic_beta.join(",");
    let mut builder = state
        .http
        .post(&url)
        .header("authorization", format!("Bearer {}", creds.access_token))
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json");

    if !beta_value.is_empty() {
        let combined = match headers.get("anthropic-beta") {
            Some(client_beta) if !client_beta.is_empty() => {
                format!("{},{}", client_beta, beta_value)
            }
            _ => beta_value.clone(),
        };
        builder = builder.header("anthropic-beta", combined);
    }

    let upstream = match builder.body(body).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, req_id, "upstream request failed");
            let _ = send
                .send(ClientMsg::RespError {
                    req_id,
                    message: format!("upstream: {}", e),
                })
                .await;
            return;
        }
    };

    let status = upstream.status().as_u16();
    let mut head_headers = HashMap::new();
    for k in ["content-type", "anthropic-request-id"] {
        if let Some(v) = upstream.headers().get(k).and_then(|v| v.to_str().ok()) {
            head_headers.insert(k.into(), v.into());
        }
    }
    if send
        .send(ClientMsg::RespHead {
            req_id,
            status,
            headers: head_headers,
        })
        .await
        .is_err()
    {
        return;
    }

    let mut stream = upstream.bytes_stream();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(b) => {
                let data_b64 = base64::engine::general_purpose::STANDARD.encode(&b);
                if send
                    .send(ClientMsg::RespChunk { req_id, data_b64 })
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Err(e) => {
                let _ = send
                    .send(ClientMsg::RespError {
                        req_id,
                        message: format!("stream: {}", e),
                    })
                    .await;
                return;
            }
        }
    }
    let _ = send.send(ClientMsg::RespEnd { req_id }).await;
}
