//! Per-workspace OS-level sandbox.
//!
//! Wraps the `claude` subprocess so it can only touch the files it needs:
//! the active workspace dir, the user's `~/.claude` credentials dir, and a
//! small set of system read-only paths. Network is left open so claude can
//! reach the Anthropic API, package registries, git remotes, etc.
//!
//! - **macOS**: Seatbelt via `sandbox_init_with_parameters` + a SBPL
//!   profile authored in this crate.
//! - **Linux**: user + mount + PID namespaces + seccomp (TODO; currently
//!   returns an unimplemented error and the agent should refuse to enable
//!   the sandbox on this platform until it lands).
//!
//! The implementation is original. The high-level approach (Seatbelt on
//! macOS, namespaces + seccomp on Linux) is the same one used by
//! Chromium's renderer sandbox, bubblewrap, and many others — that
//! pattern is a published technique, not anyone's code.

use anyhow::Result;
use std::path::PathBuf;

/// Inputs the sandbox profile interpolates into its allow rules.
#[derive(Debug, Clone)]
pub struct SandboxParams {
    /// The workspace directory `claude` will be working in. Read + write
    /// access is granted on this subtree.
    pub workspace: PathBuf,
    /// The user's home dir. The sandbox grants RW only to `~/.claude`
    /// (OAuth) and read-only access elsewhere.
    pub home: PathBuf,
}

/// Whether the workspace sandbox is implemented on this platform.
pub fn is_supported() -> bool {
    cfg!(target_os = "macos")
}

/// Apply the sandbox to the calling process. Inherits to all child
/// processes the caller spawns afterwards. Once applied it cannot be
/// removed for the lifetime of the process.
pub fn apply(params: &SandboxParams) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        macos::apply(params)
    }
    #[cfg(target_os = "linux")]
    {
        linux::apply(params)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = params;
        Err(anyhow::anyhow!(
            "workspace sandbox is not implemented on this platform"
        ))
    }
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "linux")]
mod linux;
