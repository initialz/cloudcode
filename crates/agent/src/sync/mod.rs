//! Workspace sync engine (agent side).
//!
//! When the agent holds a workspace lock, it has a local working copy at
//! `~/cloudcode-agent/workspaces/<account>/<workspace>/`. The tool
//! running inside the workspace writes files there; a filesystem
//! watcher captures every change and enqueues it into a SQLite-backed
//! push queue. A background worker
//! (added in a later phase) will drain the queue and ship
//! `WorkspacePushFile` / `WorkspaceDeleteFile` frames to the hub.
//!
//! The three submodules here are deliberately decoupled so each can be
//! unit-tested in isolation:
//!
//! - [`ignore_filter`] decides which paths the sync engine should care
//!   about — hardcoded defaults plus a per-workspace `.cloudcodeignore`.
//! - [`push_queue`] is a durable FIFO that survives agent restart.
//! - [`watcher`] bridges `notify` into a tokio mpsc, applying the ignore
//!   filter and a short coalescing window before emitting events.
//!
//! Nothing here is wired into the PtyOpen flow yet; that integration is
//! the next sub-agent's job. Until then the entire subtree is
//! technically dead code from the binary's perspective — silence the
//! linter so the build stays quiet without papering over real issues
//! once wiring lands. Drop this attribute once `pty` or `ws` spawns a
//! [`WorkspaceWatcher`] / [`PushQueue`].
#![allow(dead_code, unused_imports)]

pub mod ignore_filter;
pub mod push_queue;
pub mod watcher;

pub use ignore_filter::IgnoreFilter;
pub use push_queue::{PushQueue, QueueOp};
pub use watcher::{WatchEvent, WorkspaceWatcher};
