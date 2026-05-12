//! Linux sandbox — TODO.
//!
//! The plan: combine user / mount / PID namespaces (via `unshare(2)`),
//! bind-mount the workspace dir read-write and the rest of the rootfs
//! read-only, then install a seccomp filter that blocks a few syscall
//! classes that aren't relevant for a coding agent (kexec, bpf, …).
//!
//! For now `apply` returns an error so the agent refuses to start with
//! `[sandbox] enabled = true` on Linux. macOS is the first platform
//! supported.

use crate::sandbox::SandboxParams;
use anyhow::Result;

pub fn apply(_params: &SandboxParams) -> Result<()> {
    Err(anyhow::anyhow!(
        "workspace sandbox on Linux is not implemented yet. \
         Disable [sandbox] in agent.toml or run on macOS for now."
    ))
}
