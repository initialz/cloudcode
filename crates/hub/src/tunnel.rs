use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const PROTOCOL_VERSION: &str = "7";

// ---------------------------------------------------------------------------
// Binary frame layout (Message::Binary on the WS tunnel):
//
//   [0]      1 byte   tag (TAG_PTY_INPUT | TAG_PTY_OUTPUT)
//   [1..17]  16 bytes session_id (uuid raw bytes)
//   [17..]   payload (raw PTY bytes; no further structure)
//
// One agent connection multiplexes multiple sessions over the same WS, so
// every binary frame is keyed by session_id.
// ---------------------------------------------------------------------------

pub const TAG_PTY_INPUT: u8 = 0x01; // hub → agent : keystrokes for PTY master
pub const TAG_PTY_OUTPUT: u8 = 0x02; // agent → hub : output read from PTY master
pub const PTY_FRAME_PREFIX_LEN: usize = 1 + 16;

pub fn pack_pty_frame(tag: u8, session_id: Uuid, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(PTY_FRAME_PREFIX_LEN + payload.len());
    out.push(tag);
    out.extend_from_slice(session_id.as_bytes());
    out.extend_from_slice(payload);
    out
}

/// `(tag, session_id, payload_slice)` or None if too short / unknown tag.
pub fn unpack_pty_frame(buf: &[u8]) -> Option<(u8, Uuid, &[u8])> {
    if buf.len() < PTY_FRAME_PREFIX_LEN {
        return None;
    }
    let tag = buf[0];
    let mut sid = [0u8; 16];
    sid.copy_from_slice(&buf[1..17]);
    Some((tag, Uuid::from_bytes(sid), &buf[PTY_FRAME_PREFIX_LEN..]))
}

/// Frames sent from the agent to the hub (text JSON).
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    Hello {
        name: String,
        secret: String,
        version: String,
        /// Self-reported agent build version (`CARGO_PKG_VERSION`). Optional
        /// for compatibility with pre-v1.6.0 agents that don't send it.
        #[serde(default)]
        agent_version: Option<String>,
        /// Rust target triple of the agent binary (e.g. `aarch64-apple-darwin`).
        /// Used by the hub to pick the right release asset on self-update.
        #[serde(default)]
        target_triple: Option<String>,
        /// Names of the tools this agent is configured to spawn
        /// (auto-detected from PATH unless explicitly disabled in
        /// `agent.toml`). As of v1.13 this is effectively `["claude"]`
        /// or empty (pre-v1.13 agents). Hub forwards it to clients so
        /// they can show the right "Open" label per workspace.
        #[serde(default)]
        tools: Vec<String>,
    },
    Pong,

    /// PTY established for a session.
    PtyOpened {
        session_id: Uuid,
        workspace: String,
        cwd: String,
    },
    /// Terminal: claude or tmux exited, agent dropped the PTY, etc.
    PtyClosed {
        session_id: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// Open/runtime error that's not a normal close (couldn't spawn tmux,
    /// workspace name rejected, etc).
    PtyError {
        session_id: Uuid,
        message: String,
    },

    /// One JSONL line tailed from claude's per-project history file.
    /// The agent streams these to the hub so the admin UI can show the
    /// conversation for each session. `kind` is the outer `type` field
    /// (user / assistant / permission-mode / file-history-snapshot /
    /// …); `body` is the raw line.
    Message {
        session_id: Uuid,
        claude_session_id: String,
        ts: i64,
        kind: String,
        body: String,
    },

    /// Workspace management replies (not bound to a PTY session).
    WorkspaceListResult {
        request_id: Uuid,
        items: Vec<WorkspaceItem>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    WorkspaceCreateResult {
        request_id: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    WorkspaceDeleteResult {
        request_id: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    WorkspaceResetResult {
        request_id: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },

    /// Reply to a `WorkspaceListAll` admin query: every (account, workspace)
    /// pair this agent currently has on disk, with tmux-alive state.
    WorkspaceListAllResult {
        request_id: Uuid,
        items: Vec<WorkspaceFullItem>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },

    /// Reply to a hub-initiated `UpdateAgent` request. On success the agent
    /// exits cleanly so the supervisor relaunches it on the new binary.
    UpdateAgentResult {
        request_id: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },

    // -- v1.13 hub-managed workspace sync ----------------------------------

    /// Agent acknowledges it has consumed the `WorkspacePullStart` +
    /// `WorkspaceFile` stream for this session and is ready for the hub
    /// to send `PtyOpen` on top of the populated workspace dir.
    /// `ok=false` (with `error` set) tells the hub the pull failed and
    /// it should release the lock + propagate `SessionError` upstream.
    WorkspacePullAck {
        session_id: Uuid,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },

    /// Agent pushes one whole-file content to the hub for the
    /// session's (account, workspace). The hub writes it atomically
    /// into its `WorkspaceStorage` and replies with `WorkspaceFileAck`.
    /// Single-frame design — large files are not chunked at this
    /// layer; the sync engine on the agent side decides the size
    /// threshold above which a file is not synced at all.
    WorkspacePushFile {
        session_id: Uuid,
        path: String,
        #[serde(with = "serde_bytes")]
        content: Vec<u8>,
    },

    /// Agent reports a local delete for the (account, workspace)
    /// bound to `session_id`. Hub removes the file from its canonical
    /// copy and replies with `WorkspaceFileAck`.
    WorkspaceDeleteFile {
        session_id: Uuid,
        path: String,
    },
}

/// One row in a WorkspaceListResult. Same shape on both sides of the
/// tunnel; hub layers on `has_client` separately when forwarding to
/// the cloudcode client.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorkspaceItem {
    pub name: String,
    pub tmux_alive: bool,
}

/// Row in a `WorkspaceListAllResult`. Carries the account because the
/// admin view aggregates across all accounts the agent serves.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorkspaceFullItem {
    pub account: String,
    pub name: String,
    pub tmux_alive: bool,
}

/// Frames sent from the hub to the agent (text JSON).
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    Welcome {
        name: String,
    },
    Rejected {
        reason: RejectReason,
    },
    Ping,

    /// Allocate a PTY for a session in the given (account, workspace), with
    /// the given initial terminal size. The agent stores workspace state
    /// per-account; the tmux session name is `cloudcode-<account>-<workspace>`
    /// and the cwd is `<workspace_root>/<account>/<workspace>/`.
    PtyOpen {
        session_id: Uuid,
        account: String,
        workspace: String,
        cols: u16,
        rows: u16,
        #[serde(default)]
        claude_args: Vec<String>,
        /// Whether the agent should wrap the spawned tmux+claude in
        /// the workspace sandbox. Decided per-account on the hub
        /// (`accounts.sandbox_enabled`); the agent.toml `[sandbox]`
        /// switch is deprecated. Optional for back-compat with pre-
        /// v1.9 hubs (default false = no sandbox).
        #[serde(default)]
        sandbox: bool,
        /// Which tool to launch in the workspace. As of v1.13 this is
        /// effectively claude-only; `None` lets the agent fall back
        /// to its `[tools].default`. Kept on the wire for back-compat
        /// with pre-v1.10 hubs/clients.
        #[serde(default)]
        tool: Option<String>,
    },
    PtyResize {
        session_id: Uuid,
        cols: u16,
        rows: u16,
    },
    /// Detach this session. Does not kill the underlying tmux session — the
    /// next PtyOpen on the same (account, workspace) re-attaches.
    PtyClose {
        session_id: Uuid,
    },

    WorkspaceList {
        request_id: Uuid,
        account: String,
    },
    WorkspaceCreate {
        request_id: Uuid,
        account: String,
        name: String,
    },
    WorkspaceDelete {
        request_id: Uuid,
        account: String,
        name: String,
    },
    WorkspaceReset {
        request_id: Uuid,
        account: String,
        name: String,
    },
    /// Admin-only: ask the agent for every (account, workspace) it knows
    /// about, regardless of which account is asking. Used by the admin
    /// UI to render a cross-account workspace inventory.
    WorkspaceListAll {
        request_id: Uuid,
    },

    /// Admin-only: instruct the agent to download a new release tarball,
    /// verify its sha256, and swap the `current` symlink. On success the
    /// agent process exits cleanly and the supervisor relaunches it on
    /// the new binary.
    UpdateAgent {
        request_id: Uuid,
        /// Tag of the form `vX.Y.Z` (matches the release tag on GitHub).
        target_version: String,
        /// `.tar.gz` asset URL for this agent's target triple.
        download_url: String,
        /// `.sha256` manifest URL covering the same asset.
        sha256_url: String,
    },

    // -- v1.13 hub-managed workspace sync ----------------------------------

    /// Hub announces the start of a workspace pull for `session_id`.
    /// Agent should clear / recreate `<workspace_root>/<account>/<workspace>/`
    /// and accept the next `file_count` `WorkspaceFile` frames before the
    /// `PtyOpen` arrives. The agent MUST send a `WorkspacePullAck` after
    /// the stream terminates (signalled by `is_last=true`).
    WorkspacePullStart {
        session_id: Uuid,
        account: String,
        workspace: String,
        file_count: u64,
    },

    /// One file in the pull stream. `is_last=true` marks the final
    /// frame and ends the stream. For an empty workspace the hub
    /// still sends exactly one frame with `path=""`, empty `content`,
    /// and `is_last=true` so the agent has an unambiguous end-of-stream
    /// marker without needing to count.
    WorkspaceFile {
        session_id: Uuid,
        path: String,
        #[serde(with = "serde_bytes")]
        content: Vec<u8>,
        is_last: bool,
    },

    /// Hub acks an agent push / delete op. On `ok=false` the agent
    /// should keep the row in its push_queue and retry on the next
    /// sync tick.
    WorkspaceFileAck {
        session_id: Uuid,
        path: String,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },

    /// Sent right after `Welcome` whenever the agent has any pending
    /// "the hub force-took your lock; rm -rf your local copy"
    /// instructions queued up from being offline. Each `(account,
    /// workspace)` pair is the agent's signal to delete that local
    /// directory before it re-engages any session for that workspace.
    /// Empty `items` is never sent; if there is nothing to clean up
    /// the hub omits the frame entirely.
    WorkspaceCleanup {
        items: Vec<(String, String)>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectReason {
    NameTaken,
    AuthFailed,
    VersionMismatch,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `serde_bytes` encodes `Vec<u8>` as a JSON array of integers
    /// (not a string). The agent's mirror of `ServerMsg` uses the
    /// same attribute, so what comes out of one side has to parse on
    /// the other. This test pins the on-wire shape and confirms
    /// round-trip identity for the typical 32-byte payload size.
    #[test]
    fn workspace_file_frame_roundtrips_through_json() {
        let sid = Uuid::new_v4();
        let original = ServerMsg::WorkspaceFile {
            session_id: sid,
            path: "src/lib/foo.rs".into(),
            // Include high bytes + a NUL to confirm serde_bytes
            // doesn't truncate at a null terminator the way a
            // string-typed payload would.
            content: vec![0u8, 1, 2, 0xFE, 0xFF, b'A'],
            is_last: false,
        };
        let json = serde_json::to_string(&original).expect("encode");
        // Tag is the serde-snake-case variant name.
        assert!(json.contains(r#""type":"workspace_file""#), "got: {json}");
        assert!(json.contains(r#""is_last":false"#));
        let back: ServerMsg = serde_json::from_str(&json).expect("decode");
        match back {
            ServerMsg::WorkspaceFile {
                session_id,
                path,
                content,
                is_last,
            } => {
                assert_eq!(session_id, sid);
                assert_eq!(path, "src/lib/foo.rs");
                assert_eq!(content, vec![0u8, 1, 2, 0xFE, 0xFF, b'A']);
                assert!(!is_last);
            }
            other => panic!("decoded wrong variant: {:?}", other),
        }
    }

    /// Empty pull stream sentinel — the hub sends one frame even for
    /// a zero-file workspace. Encoding + decoding has to preserve
    /// `path=""`, `content=[]`, `is_last=true` faithfully.
    #[test]
    fn workspace_file_empty_sentinel_roundtrips() {
        let sid = Uuid::new_v4();
        let original = ServerMsg::WorkspaceFile {
            session_id: sid,
            path: String::new(),
            content: Vec::new(),
            is_last: true,
        };
        let json = serde_json::to_string(&original).unwrap();
        let back: ServerMsg = serde_json::from_str(&json).unwrap();
        match back {
            ServerMsg::WorkspaceFile {
                path,
                content,
                is_last,
                ..
            } => {
                assert!(path.is_empty());
                assert!(content.is_empty());
                assert!(is_last);
            }
            other => panic!("decoded wrong variant: {:?}", other),
        }
    }

    /// Cleanup frame carries (account, workspace) tuples — make sure
    /// the tuple shape is what the wire actually sees.
    #[test]
    fn workspace_cleanup_frame_roundtrips() {
        let original = ServerMsg::WorkspaceCleanup {
            items: vec![
                ("alice".to_string(), "demo".to_string()),
                ("bob".to_string(), "scratch".to_string()),
            ],
        };
        let json = serde_json::to_string(&original).unwrap();
        let back: ServerMsg = serde_json::from_str(&json).unwrap();
        if let ServerMsg::WorkspaceCleanup { items } = back {
            assert_eq!(items.len(), 2);
            assert_eq!(items[0], ("alice".into(), "demo".into()));
            assert_eq!(items[1], ("bob".into(), "scratch".into()));
        } else {
            panic!("decoded wrong variant: {:?}", back);
        }
    }

    /// Agent → hub push frame, including the binary payload path.
    /// Mirror of the `WorkspaceFile` test on the ClientMsg side.
    #[test]
    fn workspace_push_file_frame_roundtrips() {
        let sid = Uuid::new_v4();
        let original = ClientMsg::WorkspacePushFile {
            session_id: sid,
            path: "Cargo.toml".into(),
            content: b"[package]\nname=\"x\"\n".to_vec(),
        };
        let json = serde_json::to_string(&original).unwrap();
        assert!(json.contains(r#""type":"workspace_push_file""#));
        let back: ClientMsg = serde_json::from_str(&json).unwrap();
        match back {
            ClientMsg::WorkspacePushFile {
                session_id,
                path,
                content,
            } => {
                assert_eq!(session_id, sid);
                assert_eq!(path, "Cargo.toml");
                assert_eq!(content, b"[package]\nname=\"x\"\n".to_vec());
            }
            other => panic!("decoded wrong variant: {:?}", other),
        }
    }
}
