//! Auto-append commented-out documentation for newly-introduced
//! config keys to a binary's TOML config file on every startup.
//! Shared between hub and agent (and ready for client) — each binary
//! passes its own `[SchemaEntry]` slice describing the knobs it
//! cares about.
//!
//! Why this exists: every release we add a config knob or two. Users
//! who ran `--init` six months ago have no idea any of them exist.
//! Forcing them to grep release notes or read source for
//! `[serde(default = ...)]` attributes is silly. Instead we maintain
//! a SCHEMA list of "things you can put in this config" and on
//! startup we make sure the file has at least a commented-out
//! documentation block for each one.
//!
//! Design choices:
//!
//! * **Comments, not active values.** Defaults stay in code. The
//!   auto-block is purely educational — uncomment + edit to override.
//!   Means a user who never touches the file still gets every default
//!   value, just like before this feature existed.
//!
//! * **Dedup via the docstring's first line.** No marker fence, no
//!   "auto-generated" label — appearance in the file is the marker.
//!   On the next startup we look for the first doc line of each
//!   entry; if present (still commented or now active), we leave it
//!   alone. Delete the doc block to re-trigger.
//!
//! * **Also skip when the key is already active.** A user who ran
//!   `--init` 6 months ago has `audit_log = "..."` written by the
//!   init template; we don't want to also write a commented copy of
//!   the same key. We check the parsed TOML Document for an active
//!   value before deciding to append.
//!
//! * **Atomic write.** Read → augment → write a `.tmp` sibling →
//!   rename. A `--init` running concurrently (rare) won't see a
//!   half-written file.
//!
//! * **Soft-fail.** Anything goes wrong (file missing, parse error,
//!   read-only fs), we log a warning and let the hub start anyway.
//!   This is documentation hygiene, not a correctness requirement.

use anyhow::Result;
use std::path::Path;

/// One documented config knob. `key` is a dotted path used only for
/// the marker / dedup; `section` is the TOML table header the user
/// would need above the active assignment; `example` is the literal
/// line we write (commented) under the section.
pub struct SchemaEntry {
    pub key: &'static str,
    pub section: &'static str,
    pub doc: &'static [&'static str],
    pub example: &'static str,
}

/// Read `path`, append commented-out doc blocks for any entry in
/// `schema` that's neither already an active assignment nor already
/// auto-documented, write back atomically. Logs and returns Ok on
/// the no-changes-needed path so the caller can `.ok()` it. Pass
/// the binary's own schema slice — see hub's / agent's
/// `config_sync.rs` for examples.
pub fn sync_with_file(path: &Path, schema: &[SchemaEntry]) -> Result<()> {
    if !path.exists() {
        // --init hasn't been run yet; nothing to augment. The next
        // launch after init will append on first start.
        return Ok(());
    }
    let original = std::fs::read_to_string(path)?;
    let doc: toml_edit::DocumentMut = match original.parse() {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(file = %path.display(), error = %e, "config file unparseable; skipping schema sync");
            return Ok(());
        }
    };
    let to_append: Vec<&SchemaEntry> = missing_entries(&original, &doc, schema);
    if to_append.is_empty() {
        return Ok(());
    }
    let augmented = render_appended(&original, &to_append);
    write_atomic(path, &augmented)?;
    let names: Vec<&str> = to_append.iter().map(|e| e.key).collect();
    tracing::info!(
        added = ?names,
        "augmented {} with {} new commented config entries",
        path.display(),
        names.len()
    );
    Ok(())
}

/// Split decision out of IO so unit tests don't need a tmpfile.
fn missing_entries<'a>(
    raw: &str,
    doc: &toml_edit::DocumentMut,
    schema: &'a [SchemaEntry],
) -> Vec<&'a SchemaEntry> {
    schema
        .iter()
        .filter(|entry| !is_present(raw, doc, entry))
        .collect()
}

fn is_present(raw: &str, doc: &toml_edit::DocumentMut, entry: &SchemaEntry) -> bool {
    // Active value check: walks <section>.<leaf-key>. The doc's
    // `section.key` form means the schema's `key` is `section.leaf`;
    // strip the section prefix to get the leaf name.
    let leaf = entry
        .key
        .strip_prefix(entry.section)
        .and_then(|rest| rest.strip_prefix('.'))
        .unwrap_or(entry.key);
    if doc
        .get(entry.section)
        .and_then(|t| t.as_table())
        .and_then(|t| t.get(leaf))
        .is_some()
    {
        return true;
    }
    // Docstring dedup: if the entry's first doc line shows up
    // verbatim anywhere in the file (active or commented), assume
    // we've already added this block. Delete the doc to re-trigger
    // — a deliberate gesture, not an accident.
    entry
        .doc
        .first()
        .map(|first| raw.contains(first))
        .unwrap_or(false)
}

fn render_appended(original: &str, entries: &[&SchemaEntry]) -> String {
    let mut out = String::with_capacity(original.len() + entries.len() * 256);
    out.push_str(original);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    for entry in entries {
        out.push('\n');
        for line in entry.doc {
            out.push_str("# ");
            out.push_str(line);
            out.push('\n');
        }
        // Commented section header + assignment. User uncomments
        // both if they want to override; if the section is already
        // active above, they just uncomment the assignment and move
        // it under the existing header.
        out.push_str("# [");
        out.push_str(entry.section);
        out.push_str("]\n# ");
        out.push_str(entry.example);
        out.push('\n');
    }
    out
}

fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    // Same pattern as workspaces.rs::write_file: tmp sibling +
    // rename, so a crash mid-write doesn't leave the user with a
    // truncated config.
    let tmp = path.with_extension({
        let mut e = path
            .extension()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        e.push_str(".tmp");
        e
    });
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small fixed schema we test the engine against. Mirrors the
    /// shape of real binary schemas without coupling these tests to
    /// hub's / agent's actual key list (which is free to evolve).
    const TEST_SCHEMA: &[SchemaEntry] = &[
        SchemaEntry {
            key: "alpha.first",
            section: "alpha",
            doc: &["Alpha first doc.", "second line of alpha first doc."],
            example: r#"first = "default-a""#,
        },
        SchemaEntry {
            key: "alpha.second",
            section: "alpha",
            doc: &["Alpha second knob."],
            example: r#"second = 42"#,
        },
        SchemaEntry {
            key: "beta.path",
            section: "beta",
            doc: &["Beta path knob."],
            example: r#"path = "./beta-default""#,
        },
    ];

    fn parse(s: &str) -> toml_edit::DocumentMut {
        s.parse().unwrap()
    }

    #[test]
    fn appends_when_key_is_missing() {
        let raw = "[alpha]\nfirst = \"set\"\n";
        let doc = parse(raw);
        let missing = missing_entries(raw, &doc, TEST_SCHEMA);
        // alpha.first is active → skip; alpha.second + beta.path missing.
        assert!(!missing.iter().any(|e| e.key == "alpha.first"));
        assert!(missing.iter().any(|e| e.key == "alpha.second"));
        assert!(missing.iter().any(|e| e.key == "beta.path"));
    }

    #[test]
    fn skips_when_key_is_already_active() {
        let raw = "[alpha]\nfirst = \"x\"\nsecond = 7\n\n[beta]\npath = \"/p\"\n";
        let doc = parse(raw);
        for entry in TEST_SCHEMA {
            assert!(
                is_present(raw, &doc, entry),
                "{} should be detected as active",
                entry.key
            );
        }
    }

    #[test]
    fn skips_when_docstring_is_already_present() {
        // User left the auto block (with its first doc line) intact,
        // even though the key is still commented. Engine must dedup
        // off the docstring presence.
        let entry = &TEST_SCHEMA[2]; // beta.path
        let first_doc = entry.doc.first().unwrap();
        let raw = format!("[alpha]\nfirst = \"a\"\n\n# {first_doc}\n# [beta]\n# path = \"/zzz\"\n");
        let doc = parse(&raw);
        assert!(is_present(&raw, &doc, entry));
    }

    #[test]
    fn idempotent_across_two_runs() {
        let raw = "[alpha]\nfirst = \"x\"\n".to_string();
        let doc = parse(&raw);
        let first_pass = missing_entries(&raw, &doc, TEST_SCHEMA);
        let augmented = render_appended(&raw, &first_pass);
        let doc2 = parse(&augmented);
        let second_pass = missing_entries(&augmented, &doc2, TEST_SCHEMA);
        assert!(
            second_pass.is_empty(),
            "second sync should be a no-op, got {:?}",
            second_pass.iter().map(|e| e.key).collect::<Vec<_>>()
        );
    }

    #[test]
    fn rendered_block_round_trips_through_toml_edit() {
        let raw = "[alpha]\nfirst = \"x\"\n".to_string();
        let doc = parse(&raw);
        let entries = missing_entries(&raw, &doc, TEST_SCHEMA);
        let augmented = render_appended(&raw, &entries);
        let _re: toml_edit::DocumentMut = augmented
            .parse()
            .expect("augmented config must still parse");
    }

    #[test]
    fn rendered_block_has_doc_section_header_and_example_only() {
        let raw = String::new();
        let entries: Vec<&SchemaEntry> = TEST_SCHEMA.iter().filter(|e| e.key == "beta.path").collect();
        let out = render_appended(&raw, &entries);
        assert!(out.contains("# Beta path knob."));
        assert!(out.contains("# [beta]"));
        assert!(out.contains(r#"# path = "./beta-default""#));
        // No marker / "Section:" ceremony anymore.
        assert!(!out.contains("cloudcode-auto"));
        assert!(!out.contains("Section:"));
    }

    #[test]
    fn write_atomic_creates_tmp_and_renames() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("hub.toml");
        std::fs::write(&target, "[alpha]\nfirst = \"x\"\n").unwrap();
        write_atomic(&target, "new\ncontent\n").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new\ncontent\n");
        // tmp must NOT survive the rename.
        assert!(!dir.path().join("hub.toml.tmp").exists());
    }

    #[test]
    fn missing_file_is_a_silent_no_op() {
        let dir = tempfile::tempdir().unwrap();
        sync_with_file(&dir.path().join("nope.toml"), TEST_SCHEMA).unwrap();
    }
}
