//! Client-specific schema for the auto-sync engine.
//!
//! Currently empty: the client config only carries `hub_url` and
//! `token`, both REQUIRED — there are no optional knobs to document.
//! Wiring the framework anyway means adding a future option is a
//! one-line edit here, no plumbing changes.

use cloudcode_daemon::config_sync::SchemaEntry;

pub const SCHEMA: &[SchemaEntry] = &[];
