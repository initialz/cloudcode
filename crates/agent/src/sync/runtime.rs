//! Per-session push worker that bridges the watcher + push queue to
//! the WS tunnel.
//!
//! One [`run_push_worker`] task is spawned per active PtyOpen (i.e.
//! per (session_id, account, workspace)). It owns three inputs:
//!
//! 1. The watcher's `WatchEvent` receiver — turns filesystem changes
//!    into [`QueueOp`]s and persists them via [`PushQueue::enqueue`]
//!    after coalescing redundant entries on the same path.
//! 2. A periodic queue scan — pulls the oldest `PEEK_BATCH` ops not
//!    yet "in flight" and ships them over the WS tx, recording the
//!    queue id in a `pending` map keyed by path.
//! 3. An ack channel — the WS read loop forwards every
//!    `WorkspaceFileAck` for this session here so the worker can
//!    `queue.ack(id)` the matching row.
//!
//! Why one task instead of three?
//!   - The `pending` map is the only piece of mutable state shared
//!     between "ship a frame" and "receive an ack". Keeping it in one
//!     task means no `Mutex` / `RwLock` — `tokio::select!` on the
//!     three sources is enough.
//!   - It also means cancellation is trivial: drop the shutdown
//!     `mpsc::Sender` and the task exits on the next select! tick.
//!
//! Reliability notes:
//!   - The queue is durable. If the agent crashes or the WS drops
//!     mid-send, the next worker boot picks up where the old one
//!     stopped — `pending` is in-memory, but the row is still in
//!     SQLite, so the worst case is one duplicate push per outstanding
//!     ack (hub-side is idempotent on `(account, workspace, path)`).
//!   - On `ok = false` we leave the row alone; the next scan will
//!     resend it. Worst case is a tight loop on a permanently-failing
//!     push; we mitigate that with an exponential backoff between
//!     scans whenever the last scan saw any failures.

use crate::pty::OutFrame;
use crate::sync::push_queue::{PushQueue, QueueOp};
use crate::sync::watcher::WatchEvent;
use crate::tunnel::ClientMsg;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

/// Result of a single push ack, routed by the WS read loop to the
/// worker that owns the session.
#[derive(Debug, Clone)]
pub struct AckMsg {
    pub path: String,
    pub ok: bool,
    pub error: Option<String>,
}

/// How many rows the worker drags out of SQLite per scan.
const PEEK_BATCH: usize = 50;
/// Quiet poll cadence when there's nothing to send / no events
/// arriving. The watcher pushes events through the same select, so
/// this only matters as a backstop in case the watcher misses or
/// the queue still has entries from a previous session.
const SCAN_INTERVAL: Duration = Duration::from_millis(500);
/// Backoff applied after a scan that saw at least one `ok = false`
/// ack — keeps a permanently-failing push from busy-looping the WS.
const BACKOFF_AFTER_FAILURE: Duration = Duration::from_secs(2);

/// Inputs the worker drives. Kept as a struct so `PtyManager` can
/// build the bundle once and hand it off into `tokio::spawn`.
pub struct PushWorker {
    pub session_id: Uuid,
    pub account: String,
    pub workspace: String,
    pub workspace_root: PathBuf,
    pub queue: Arc<PushQueue>,
    pub watch_rx: mpsc::Receiver<WatchEvent>,
    pub ack_rx: mpsc::Receiver<AckMsg>,
    pub shutdown_rx: mpsc::Receiver<()>,
    pub tx: mpsc::Sender<OutFrame>,
}

/// Drive the loop until shutdown / channels drained.
///
/// The task exits when:
///   - the shutdown channel fires (PtyClose / reader EOF), OR
///   - the WS tx is closed (writer task gone — hub disconnect), OR
///   - both the watcher and ack channels are closed (caller dropped
///     everything; treat the same as shutdown).
pub async fn run_push_worker(mut w: PushWorker) {
    let mut pending: HashMap<String, u64> = HashMap::new();
    let mut tick = tokio::time::interval(SCAN_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut next_scan_delay: Option<Duration> = None;

    loop {
        // Honour any pending backoff. We use a one-shot sleep instead of
        // mutating the interval so a watcher event during the backoff
        // still wakes us promptly.
        let backoff_sleep: futures::future::BoxFuture<()> = match next_scan_delay.take() {
            Some(d) => Box::pin(tokio::time::sleep(d)),
            None => Box::pin(futures::future::pending()),
        };
        tokio::select! {
            _ = w.shutdown_rx.recv() => {
                tracing::debug!(session = %w.session_id, "push worker: shutdown");
                break;
            }
            evt = w.watch_rx.recv() => {
                let Some(evt) = evt else {
                    // Watcher gone -> sync engine stopped; we can still
                    // drain whatever's in the queue from acks coming in,
                    // but nothing fresh will arrive. Keep going until
                    // queue + ack channel are both empty.
                    if !drain_remaining(&mut w, &mut pending).await {
                        break;
                    }
                    continue;
                };
                if let Err(e) = handle_watch_event(&w, evt).await {
                    tracing::warn!(session = %w.session_id, error = %e, "push worker: handle watch event");
                }
                // Falls through to the next select cycle; the next tick
                // (or this iteration's `if scan_due` below) will pick up
                // the newly-enqueued row.
            }
            ack = w.ack_rx.recv() => {
                let Some(ack) = ack else {
                    // The session is being torn down by ws read loop.
                    // Stop accepting new acks but keep the worker alive
                    // for shutdown_rx to fire.
                    continue;
                };
                if let Some(id) = pending.remove(&ack.path) {
                    if ack.ok {
                        if let Err(e) = w.queue.ack(id).await {
                            tracing::warn!(session = %w.session_id, error = %e, "push worker: queue.ack");
                        }
                    } else {
                        tracing::warn!(
                            session = %w.session_id,
                            path = %ack.path,
                            err = ?ack.error,
                            "push worker: ack reported failure; will retry"
                        );
                        next_scan_delay = Some(BACKOFF_AFTER_FAILURE);
                    }
                } else {
                    tracing::debug!(
                        session = %w.session_id,
                        path = %ack.path,
                        "push worker: ack for unknown path (already dequeued?)"
                    );
                }
            }
            _ = backoff_sleep => {
                // Backoff elapsed; fall through to the scan tick below
                // by re-arming the interval and continuing.
                tick.reset();
            }
            _ = tick.tick() => {
                if let Err(e) = scan_and_send(&mut w, &mut pending).await {
                    tracing::warn!(session = %w.session_id, error = %e, "push worker: scan");
                }
            }
        }
    }

    tracing::debug!(session = %w.session_id, "push worker: exiting");
}

/// Persist one watcher event in the queue (with coalescing). Reads
/// file contents lazily so removed files don't trip on a missing read.
async fn handle_watch_event(w: &PushWorker, evt: WatchEvent) -> anyhow::Result<()> {
    let (path, op) = match evt {
        WatchEvent::Changed { path } => {
            let abs = w
                .workspace_root
                .join(&w.account)
                .join(&w.workspace)
                .join(&path);
            // Read the file before enqueueing. If the file is gone by
            // the time we get here (race between event and read), treat
            // it as a delete instead — the watcher will catch up on
            // the next sweep.
            match tokio::fs::read(&abs).await {
                Ok(bytes) => {
                    let p = path.to_string_lossy().to_string();
                    (
                        p.clone(),
                        QueueOp::PushFile {
                            account: w.account.clone(),
                            workspace: w.workspace.clone(),
                            path: p,
                            content: bytes,
                        },
                    )
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    let p = path.to_string_lossy().to_string();
                    (
                        p.clone(),
                        QueueOp::DeleteFile {
                            account: w.account.clone(),
                            workspace: w.workspace.clone(),
                            path: p,
                        },
                    )
                }
                Err(e) => return Err(e.into()),
            }
        }
        WatchEvent::Removed { path } => {
            let p = path.to_string_lossy().to_string();
            (
                p.clone(),
                QueueOp::DeleteFile {
                    account: w.account.clone(),
                    workspace: w.workspace.clone(),
                    path: p,
                },
            )
        }
    };

    w.queue.enqueue(op).await?;
    // Drop any older rows for this path now that the newest one is in.
    let _ = w
        .queue
        .coalesce_path(&w.account, &w.workspace, &path)
        .await?;
    Ok(())
}

/// Pull the oldest queue rows for this `(account, workspace)` and
/// ship anything that's not currently in flight. Returns Ok(()) on
/// success — the count is logged at trace level only.
async fn scan_and_send(
    w: &mut PushWorker,
    pending: &mut HashMap<String, u64>,
) -> anyhow::Result<()> {
    let rows = w.queue.peek_oldest(PEEK_BATCH).await?;
    for (id, op) in rows {
        // Filter to this session's (account, workspace). The queue is
        // shared per-agent, so we may see other sessions' rows here.
        let (op_account, op_workspace, path) = match &op {
            QueueOp::PushFile {
                account,
                workspace,
                path,
                ..
            }
            | QueueOp::DeleteFile {
                account,
                workspace,
                path,
            } => (account.as_str(), workspace.as_str(), path.as_str()),
        };
        if op_account != w.account || op_workspace != w.workspace {
            continue;
        }
        if pending.contains_key(path) {
            // Already shipped, awaiting ack.
            continue;
        }
        // Send and remember.
        let frame = match &op {
            QueueOp::PushFile {
                path, content, ..
            } => ClientMsg::WorkspacePushFile {
                session_id: w.session_id,
                path: path.clone(),
                content: content.clone(),
            },
            QueueOp::DeleteFile { path, .. } => ClientMsg::WorkspaceDeleteFile {
                session_id: w.session_id,
                path: path.clone(),
            },
        };
        if w.tx.send(OutFrame::Text(frame)).await.is_err() {
            // Writer task gone; stop scanning. The shutdown signal
            // will arrive soon. Don't mark this row as pending — next
            // session's worker will pick it up.
            return Ok(());
        }
        pending.insert(path.to_string(), id);
    }
    Ok(())
}

/// After the watcher channel closes, keep the worker alive long
/// enough to receive in-flight acks. Returns `true` if there's still
/// work to do (so the outer loop should continue), `false` if the
/// queue is empty and no acks are pending.
async fn drain_remaining(
    w: &mut PushWorker,
    pending: &mut HashMap<String, u64>,
) -> bool {
    // Cheap check: if we have nothing pending and the queue has
    // nothing for us, stop.
    let len_for_us = match w.queue.peek_oldest(1).await {
        Ok(rows) => rows.iter().any(|(_, op)| {
            matches!(
                op,
                QueueOp::PushFile { account, workspace, .. }
                | QueueOp::DeleteFile { account, workspace, .. }
                if account == &w.account && workspace == &w.workspace
            )
        }),
        Err(_) => false,
    };
    !pending.is_empty() || len_for_us
}
