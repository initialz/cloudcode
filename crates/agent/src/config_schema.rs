//! Agent-specific list of optional config knobs the auto-sync engine
//! should keep documented in `agent.toml`. Each entry produces a
//! commented-out doc + assignment block at the end of the file the
//! first time a release introduces it.
//!
//! Add new entries at the BOTTOM so diff-across-releases stays
//! readable. Required fields (`[hub].url`, `[auth].registration_token`)
//! are not listed — they have no defaults and the agent refuses to
//! start without them, so a missing key is already loud.

use cloudcode_daemon::config_sync::SchemaEntry;

pub const SCHEMA: &[SchemaEntry] = &[
    SchemaEntry {
        key: "agent.name",
        section: "agent",
        doc: &[
            "Human-readable name this agent registers under. Used by the",
            "hub to address it (admin UI, client agent picker). If unset,",
            "the agent registers with the machine's hostname.",
        ],
        example: r#"name = "laptop-pete""#,
    },
    SchemaEntry {
        key: "claude.workspace_root",
        section: "claude",
        doc: &[
            "Per-workspace dirs land under this root. Relative to the",
            "agent's working directory by default. Set an absolute path",
            "to put workspaces on a different volume from the agent",
            "binary / config. Default: ./agent/workspaces",
        ],
        example: r#"workspace_root = "./agent/workspaces""#,
    },
    SchemaEntry {
        key: "claude.executable",
        section: "claude",
        doc: &[
            "Argv0 used when launching claude. Override to point at a",
            "wrapper that injects env vars / a mise / direnv shim.",
            "Default: claude (looked up via PATH).",
        ],
        example: r#"executable = "claude""#,
    },
    SchemaEntry {
        key: "claude.extra_args",
        section: "claude",
        doc: &[
            "Extra args appended after `claude` on every launch.",
            "Default: [] (none).",
        ],
        example: r#"extra_args = []"#,
    },
    SchemaEntry {
        key: "tmux.executable",
        section: "tmux",
        doc: &[
            "tmux binary used by the agent's PTY layer. Looked up via",
            "PATH unless an absolute path. Default: tmux",
        ],
        example: r#"executable = "tmux""#,
    },
    SchemaEntry {
        key: "sandbox.enabled",
        section: "sandbox",
        doc: &[
            "Wrap each claude (and its tmux session) in a per-workspace",
            "OS-level sandbox. macOS only at the moment. Default: false",
        ],
        example: r#"enabled = false"#,
    },
    SchemaEntry {
        key: "recording.dir",
        section: "recording",
        doc: &[
            "Where asciinema *.cast files land. Default:",
            "  ~/.local/state/cloudcode/agent/recordings",
        ],
        example: r#"dir = "~/.local/state/cloudcode/agent/recordings""#,
    },
    SchemaEntry {
        key: "recording.keep_days",
        section: "recording",
        doc: &[
            "Recordings older than this are eligible for garbage",
            "collection. 0 (default) keeps them forever.",
        ],
        example: r#"keep_days = 0"#,
    },
];
