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
//! As of v1.13 Round 2 the engine is wired into the PtyOpen flow: when
//! the hub announces a workspace pull, [`crate::pty::PtyManager`] writes
//! the canonical files to disk, then spawns a [`WorkspaceWatcher`] plus
//! a per-session [`runtime::run_push_worker`] task that drains pushes
//! through the shared [`PushQueue`].
#![allow(unused_imports)]

pub mod ignore_filter;
pub mod push_queue;
pub mod runtime;
pub mod watcher;

pub use ignore_filter::IgnoreFilter;
pub use push_queue::{PushQueue, QueueOp};
pub use runtime::{run_push_worker, AckMsg, PushWorker};
pub use watcher::{WatchEvent, WorkspaceWatcher};
