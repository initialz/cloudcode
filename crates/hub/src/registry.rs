use crate::tunnel::{ClientMsg, ServerMsg};
use base64::Engine;
use bytes::Bytes;
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};

static NEXT_CONN_ID: AtomicU64 = AtomicU64::new(1);

pub struct AgentRegistry {
    agents: DashMap<String, Arc<AgentConn>>,
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self {
            agents: DashMap::new(),
        }
    }

    /// Atomically register `name`. Returns None if a connection with this
    /// name is already registered (caller should send Rejected::NameTaken).
    pub fn try_register(
        self: &Arc<Self>,
        name: String,
        send: mpsc::Sender<ServerMsg>,
    ) -> Option<Arc<AgentConn>> {
        let entry = self.agents.entry(name.clone());
        match entry {
            dashmap::mapref::entry::Entry::Occupied(_) => None,
            dashmap::mapref::entry::Entry::Vacant(v) => {
                let conn = Arc::new(AgentConn {
                    name,
                    id: NEXT_CONN_ID.fetch_add(1, Ordering::Relaxed),
                    send,
                    next_req_id: AtomicU64::new(1),
                    inflight: DashMap::new(),
                });
                v.insert(conn.clone());
                Some(conn)
            }
        }
    }

    /// Remove the entry only if it still matches `conn`'s id. This avoids
    /// a stale reader loop deleting a fresh connection of the same name.
    pub fn unregister(&self, conn: &AgentConn) {
        let should_remove = self
            .agents
            .get(&conn.name)
            .map(|e| e.value().id == conn.id)
            .unwrap_or(false);
        if should_remove {
            self.agents.remove(&conn.name);
        }
        // Fail any in-flight requests so callers wake up with an error.
        for entry in conn.inflight.iter() {
            if let Some(tx) = entry.value().head.lock().unwrap().take() {
                let _ = tx.send(Err("agent disconnected".into()));
            }
            let _ = entry
                .value()
                .body
                .try_send(Err("agent disconnected".into()));
        }
        conn.inflight.clear();
    }

    pub fn get(&self, name: &str) -> Option<Arc<AgentConn>> {
        self.agents.get(name).map(|e| e.value().clone())
    }
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub struct AgentConn {
    pub name: String,
    id: u64,
    send: mpsc::Sender<ServerMsg>,
    next_req_id: AtomicU64,
    inflight: DashMap<u64, InflightSlot>,
}

struct InflightSlot {
    head: Mutex<Option<oneshot::Sender<Result<HeadOk, String>>>>,
    body: mpsc::Sender<Result<Bytes, String>>,
}

pub struct HeadOk {
    pub status: u16,
    pub headers: HashMap<String, String>,
}

pub struct ForwardRequest {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

pub struct InflightResponse {
    pub head: oneshot::Receiver<Result<HeadOk, String>>,
    pub body: mpsc::Receiver<Result<Bytes, String>>,
    pub guard: InflightGuard,
}

/// Removes the in-flight slot from the agent's table when dropped. Keep it
/// alive until the body stream finishes, otherwise late chunks would land in
/// a vacant slot and be dropped silently.
pub struct InflightGuard {
    conn: Arc<AgentConn>,
    req_id: u64,
}

impl Drop for InflightGuard {
    fn drop(&mut self) {
        self.conn.inflight.remove(&self.req_id);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    #[error("agent disconnected")]
    Disconnected,
}

impl AgentConn {
    pub async fn dispatch(
        self: &Arc<Self>,
        req: ForwardRequest,
    ) -> Result<InflightResponse, DispatchError> {
        let req_id = self.next_req_id.fetch_add(1, Ordering::Relaxed);
        let (head_tx, head_rx) = oneshot::channel();
        let (body_tx, body_rx) = mpsc::channel(64);
        self.inflight.insert(
            req_id,
            InflightSlot {
                head: Mutex::new(Some(head_tx)),
                body: body_tx,
            },
        );
        let msg = ServerMsg::Request {
            req_id,
            method: req.method,
            path: req.path,
            headers: req.headers,
            body_b64: base64::engine::general_purpose::STANDARD.encode(&req.body),
        };
        if self.send.send(msg).await.is_err() {
            self.inflight.remove(&req_id);
            return Err(DispatchError::Disconnected);
        }
        Ok(InflightResponse {
            head: head_rx,
            body: body_rx,
            guard: InflightGuard {
                conn: self.clone(),
                req_id,
            },
        })
    }

    /// Apply an incoming agent frame to in-flight state.
    pub fn handle_frame(&self, frame: ClientMsg) {
        match frame {
            ClientMsg::RespHead {
                req_id,
                status,
                headers,
            } => {
                if let Some(slot) = self.inflight.get(&req_id) {
                    if let Some(tx) = slot.head.lock().unwrap().take() {
                        let _ = tx.send(Ok(HeadOk { status, headers }));
                    }
                }
            }
            ClientMsg::RespChunk { req_id, data_b64 } => {
                let Ok(bytes) =
                    base64::engine::general_purpose::STANDARD.decode(data_b64.as_bytes())
                else {
                    return;
                };
                if let Some(slot) = self.inflight.get(&req_id) {
                    let _ = slot.body.try_send(Ok(Bytes::from(bytes)));
                }
            }
            ClientMsg::RespEnd { req_id } => {
                // Drop the body sender so the receiver stream completes.
                self.inflight.remove(&req_id);
            }
            ClientMsg::RespError { req_id, message } => {
                if let Some((_, slot)) = self.inflight.remove(&req_id) {
                    if let Some(tx) = slot.head.lock().unwrap().take() {
                        let _ = tx.send(Err(message));
                    } else {
                        let _ = slot.body.try_send(Err(message));
                    }
                }
            }
            ClientMsg::Hello { .. } | ClientMsg::Pong => {}
        }
    }
}
