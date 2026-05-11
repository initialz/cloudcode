use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const PROTOCOL_VERSION: &str = "1";

/// Frames sent from the agent to the hub.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    Hello {
        name: String,
        secret: String,
        version: String,
    },
    RespHead {
        req_id: u64,
        status: u16,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
    RespChunk {
        req_id: u64,
        data_b64: String,
    },
    RespEnd {
        req_id: u64,
    },
    RespError {
        req_id: u64,
        message: String,
    },
    Pong,
}

/// Frames sent from the hub to the agent.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    Welcome {
        name: String,
    },
    Rejected {
        reason: RejectReason,
    },
    Request {
        req_id: u64,
        method: String,
        path: String,
        #[serde(default)]
        headers: HashMap<String, String>,
        body_b64: String,
    },
    Ping,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectReason {
    NameInvalid,
    NameTaken,
    AuthFailed,
    VersionMismatch,
}
