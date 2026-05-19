//! SQLite-backed persistence for accounts, audit, and session tracking.
//!
//! The hub used to keep accounts inline in `hub.toml` and audit events
//! in an append-only JSONL file. Both have moved to a SQLite db so the
//! admin UI can query, filter, and aggregate them. The JSONL audit log
//! is kept as a secondary sink (append-only durability + offline
//! analysis).
//!
//! Single-file db, WAL mode, ~8 connection pool. SQLite is plenty for
//! the kind of write rate a cloudcode hub sees (a few events per session,
//! maybe a few accounts).

use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::path::Path;
use std::str::FromStr;

#[derive(Clone)]
pub struct Db {
    pub pool: SqlitePool,
}

#[derive(Debug, Clone)]
pub struct DbAccount {
    pub name: String,
    pub token_hash: String,
    pub token_prefix: Option<String>,
    pub created_at: i64,
    pub disabled: bool,
    /// Per-account workspace sandbox toggle. Defaults to true on
    /// fresh installs; existing accounts get true at migration time.
    /// Replaces the agent.toml-level `[sandbox] enabled` switch.
    pub sandbox_enabled: bool,
}

impl Db {
    pub async fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating db dir {}", parent.display()))?;
            }
        }
        let dsn = format!("sqlite://{}", path.display());
        let opts = SqliteConnectOptions::from_str(&dsn)?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(std::time::Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(opts)
            .await
            .with_context(|| format!("opening sqlite at {}", path.display()))?;
        let db = Self { pool };
        db.run_migrations().await?;
        Ok(db)
    }

    async fn run_migrations(&self) -> Result<()> {
        // No external migration tool — hub owns its schema. Each statement
        // is idempotent (`IF NOT EXISTS`) so an existing db just gets the
        // new objects on upgrade.
        let stmts = [
            "CREATE TABLE IF NOT EXISTS accounts (
                name            TEXT PRIMARY KEY,
                token_hash      TEXT NOT NULL,
                token_prefix    TEXT,
                created_at      INTEGER NOT NULL,
                disabled        INTEGER NOT NULL DEFAULT 0,
                sandbox_enabled INTEGER NOT NULL DEFAULT 1
            )",
            // Idempotent ALTER for deployments that pre-date the
            // sandbox_enabled column. SQLite errors on duplicate
            // column; the next statement swallows that case via
            // the marker check below.
            "ALTER TABLE accounts ADD COLUMN sandbox_enabled INTEGER NOT NULL DEFAULT 1",
            "CREATE TABLE IF NOT EXISTS audit_events (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                ts         INTEGER NOT NULL,
                kind       TEXT NOT NULL,
                account    TEXT,
                agent      TEXT,
                session_id TEXT,
                workspace  TEXT,
                detail     TEXT
            )",
            "CREATE INDEX IF NOT EXISTS idx_audit_ts ON audit_events(ts DESC)",
            "CREATE INDEX IF NOT EXISTS idx_audit_account ON audit_events(account)",
            "CREATE INDEX IF NOT EXISTS idx_audit_kind ON audit_events(kind)",
            "CREATE TABLE IF NOT EXISTS sessions (
                session_id  TEXT PRIMARY KEY,
                account     TEXT NOT NULL,
                agent       TEXT NOT NULL,
                workspace   TEXT NOT NULL,
                started_at  INTEGER NOT NULL,
                ended_at    INTEGER,
                ended_reason TEXT
            )",
            "CREATE INDEX IF NOT EXISTS idx_sessions_started ON sessions(started_at DESC)",
            "CREATE INDEX IF NOT EXISTS idx_sessions_active ON sessions(ended_at) WHERE ended_at IS NULL",
            // Conversation messages tailed from claude's per-project
            // jsonl logs. One row per JSONL line; `kind` is the outer
            // `type` field (user / assistant / permission-mode / ...);
            // `body` is the raw line as JSON.
            "CREATE TABLE IF NOT EXISTS messages (
                id                INTEGER PRIMARY KEY AUTOINCREMENT,
                cc_session_id     TEXT NOT NULL,
                claude_session_id TEXT NOT NULL,
                ts                INTEGER NOT NULL,
                kind              TEXT NOT NULL,
                body              TEXT NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS idx_messages_cc_session ON messages(cc_session_id, id)",
            "CREATE INDEX IF NOT EXISTS idx_messages_claude_session ON messages(claude_session_id, id)",
            "CREATE INDEX IF NOT EXISTS idx_messages_ts ON messages(ts DESC)",
            // Track each jsonl file's byte offset so the agent can resume
            // tailing where it left off after a restart. Keyed on the
            // claude session id (== filename without .jsonl); the cc
            // session that first saw it is recorded for routing.
            "CREATE TABLE IF NOT EXISTS jsonl_progress (
                claude_session_id TEXT PRIMARY KEY,
                cc_session_id     TEXT NOT NULL,
                offset            INTEGER NOT NULL,
                updated_at        INTEGER NOT NULL
            )",
            // Per-account whitelist of agents this account may connect to.
            // Semantics: strict whitelist — a row must exist for the
            // (account, agent) pair, otherwise the account is denied.
            // First-run seed (below) grants each pre-existing account
            // every agent it had historically connected to (derived from
            // sessions), so v0.9 upgrades didn't lock anyone out.
            "CREATE TABLE IF NOT EXISTS account_allowed_agents (
                account TEXT NOT NULL,
                agent   TEXT NOT NULL,
                PRIMARY KEY (account, agent)
            )",
            // Key-value scratchpad for migrations that need to run
            // exactly once across the lifetime of the database. Without
            // this table the ACL seed below would re-run on every hub
            // restart and resurrect rows the admin had explicitly
            // deleted from the UI.
            "CREATE TABLE IF NOT EXISTS db_meta (
                key   TEXT PRIMARY KEY,
                value TEXT
            )",
            // Per-account opaque JSON blob holding webterm UI preferences
            // (default args per tool, future things). The hub does not
            // interpret `data`; webterm owns the schema and migrates the
            // blob in code as it evolves. One row per account.
            "CREATE TABLE IF NOT EXISTS user_preferences (
                account    TEXT PRIMARY KEY,
                data       TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            )",
            // Persistent admin / webterm session stores. Previously
            // both sat in in-process DashMaps and got wiped on every
            // hub restart, forcing operators + webterm users to log
            // back in after each upgrade. Now the only thing
            // restart-sensitive is the cookie's expiration timestamp.
            "CREATE TABLE IF NOT EXISTS admin_sessions (
                sid        TEXT PRIMARY KEY,
                expires_at INTEGER NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS idx_admin_sessions_expires ON admin_sessions(expires_at)",
            "CREATE TABLE IF NOT EXISTS user_sessions (
                sid        TEXT PRIMARY KEY,
                account    TEXT NOT NULL,
                expires_at INTEGER NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS idx_user_sessions_expires ON user_sessions(expires_at)",
            // Hub-managed workspaces (v1.13). The hub holds the canonical
            // copy of a workspace's files under <state>/hub/workspaces/;
            // agents pull a working copy and need a hub-side lock to
            // push changes. `locked_by_agent` / `locked_at` are NULL
            // when nobody owns the workspace. `size_bytes` is the
            // aggregate size of the canonical copy, refreshed by the
            // sync engine on each successful push.
            "CREATE TABLE IF NOT EXISTS workspaces (
                account            TEXT NOT NULL,
                name               TEXT NOT NULL,
                created_at         INTEGER NOT NULL,
                locked_by_agent    TEXT,
                locked_at          INTEGER,
                last_sync_at       INTEGER,
                size_bytes         INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (account, name)
            )",
            "CREATE INDEX IF NOT EXISTS idx_workspaces_locked_by ON workspaces(locked_by_agent)",
            // When the hub force-takes a lock from an offline agent we
            // queue a "rm -rf local copy" instruction here. The agent
            // drains its row on the next hello frame and applies it
            // before reconnecting to any sessions.
            "CREATE TABLE IF NOT EXISTS pending_workspace_cleanups (
                agent      TEXT NOT NULL,
                account    TEXT NOT NULL,
                workspace  TEXT NOT NULL,
                queued_at  INTEGER NOT NULL,
                PRIMARY KEY (agent, account, workspace)
            )",
            // Compat for deployments that already ran the unguarded
            // seed (pre-v1.8.x): if the ACL table is non-empty, assume
            // the seed has logically happened and lock the marker in,
            // so the WHERE NOT EXISTS guard below short-circuits and
            // we don't undelete anything on the next start.
            "INSERT OR IGNORE INTO db_meta (key, value)
                SELECT 'seeded_acl_v0_9', '1'
                 WHERE EXISTS (SELECT 1 FROM account_allowed_agents)",
            // Fresh deployments: actually run the seed. Guarded so it
            // only happens once.
            "INSERT OR IGNORE INTO account_allowed_agents (account, agent)
                SELECT DISTINCT s.account, s.agent
                  FROM sessions s
                  JOIN accounts a ON a.name = s.account
                 WHERE NOT EXISTS (
                     SELECT 1 FROM db_meta WHERE key = 'seeded_acl_v0_9'
                 )",
            // Lock the marker in unconditionally — even for fresh dbs
            // with zero historical sessions, so the seed doesn't try
            // again once the admin starts using the system.
            "INSERT OR IGNORE INTO db_meta (key, value)
                VALUES ('seeded_acl_v0_9', '1')",
        ];
        for sql in stmts {
            let res = sqlx::query(sql).execute(&self.pool).await;
            if let Err(e) = res {
                // Idempotent ALTER TABLE: SQLite returns "duplicate
                // column name" when the column already exists. Treat
                // that as success so re-running migrations on an
                // already-upgraded db is a no-op.
                let msg = e.to_string();
                if msg.contains("duplicate column name") {
                    continue;
                }
                return Err(e).with_context(|| {
                    format!(
                        "migrate: {}",
                        sql.split_whitespace().take(4).collect::<Vec<_>>().join(" ")
                    )
                });
            }
        }
        Ok(())
    }

    // ---- accounts ------------------------------------------------------

    pub async fn account_count(&self) -> Result<i64> {
        let row = sqlx::query("SELECT COUNT(*) AS n FROM accounts")
            .fetch_one(&self.pool)
            .await?;
        Ok(row.get::<i64, _>("n"))
    }

    /// Per-account activity rollup: most recent session start (or
    /// None if the account has never opened one) and how many of
    /// its sessions are currently live (ended_at IS NULL). One SQL
    /// round trip; admin UI uses it next to list_accounts() to
    /// render Online / Last used columns without N+1ing.
    pub async fn account_activity_index(
        &self,
    ) -> Result<Vec<(String, Option<i64>, i64)>> {
        let rows = sqlx::query(
            "SELECT account,
                    MAX(started_at) AS last_used,
                    SUM(CASE WHEN ended_at IS NULL THEN 1 ELSE 0 END) AS active_count
               FROM sessions
              GROUP BY account",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                (
                    r.get::<String, _>("account"),
                    r.get::<Option<i64>, _>("last_used"),
                    r.get::<Option<i64>, _>("active_count").unwrap_or(0),
                )
            })
            .collect())
    }

    pub async fn list_accounts(&self) -> Result<Vec<DbAccount>> {
        let rows = sqlx::query(
            "SELECT name, token_hash, token_prefix, created_at, disabled, sandbox_enabled
             FROM accounts ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| DbAccount {
                name: r.get("name"),
                token_hash: r.get("token_hash"),
                token_prefix: r.get("token_prefix"),
                created_at: r.get("created_at"),
                disabled: r.get::<i64, _>("disabled") != 0,
                sandbox_enabled: r.get::<i64, _>("sandbox_enabled") != 0,
            })
            .collect())
    }

    /// Look up a single account's sandbox toggle. Default true if the
    /// account is missing (the OpenSession handler still validates the
    /// account before this is consulted; missing here means the row
    /// vanished between auth and PtyOpen — better to err on the side
    /// of more isolation).
    pub async fn account_sandbox_enabled(&self, name: &str) -> Result<bool> {
        let row = sqlx::query(
            "SELECT sandbox_enabled FROM accounts WHERE name = ?1",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row
            .map(|r| r.get::<i64, _>("sandbox_enabled") != 0)
            .unwrap_or(true))
    }

    pub async fn set_account_sandbox(&self, name: &str, enabled: bool) -> Result<()> {
        let rows = sqlx::query("UPDATE accounts SET sandbox_enabled = ?1 WHERE name = ?2")
            .bind(if enabled { 1_i64 } else { 0_i64 })
            .bind(name)
            .execute(&self.pool)
            .await?
            .rows_affected();
        if rows == 0 {
            anyhow::bail!("account '{}' not found", name);
        }
        Ok(())
    }

    pub async fn insert_account(
        &self,
        name: &str,
        token_hash: &str,
        token_prefix: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO accounts (name, token_hash, token_prefix, created_at, disabled)
             VALUES (?1, ?2, ?3, ?4, 0)",
        )
        .bind(name)
        .bind(token_hash)
        .bind(token_prefix)
        .bind(chrono::Utc::now().timestamp())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn account_exists(&self, name: &str) -> Result<bool> {
        let row = sqlx::query("SELECT 1 AS one FROM accounts WHERE name = ?1")
            .bind(name)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }

    pub async fn update_account_token(
        &self,
        name: &str,
        token_hash: &str,
        token_prefix: &str,
    ) -> Result<()> {
        let rows = sqlx::query(
            "UPDATE accounts SET token_hash = ?1, token_prefix = ?2 WHERE name = ?3",
        )
        .bind(token_hash)
        .bind(token_prefix)
        .bind(name)
        .execute(&self.pool)
        .await?
        .rows_affected();
        if rows == 0 {
            anyhow::bail!("account '{}' not found", name);
        }
        Ok(())
    }

    pub async fn set_account_disabled(&self, name: &str, disabled: bool) -> Result<()> {
        let rows = sqlx::query("UPDATE accounts SET disabled = ?1 WHERE name = ?2")
            .bind(if disabled { 1_i64 } else { 0_i64 })
            .bind(name)
            .execute(&self.pool)
            .await?
            .rows_affected();
        if rows == 0 {
            anyhow::bail!("account '{}' not found", name);
        }
        Ok(())
    }

    pub async fn delete_account(&self, name: &str) -> Result<()> {
        // No SQLite foreign keys here, so we walk the dependent
        // tables ourselves. Drop the ACL rows first so a partial
        // failure still leaves the world in a consistent state
        // (orphan ACL rows are worse than orphan audit rows —
        // sessions history is meant to outlive the account).
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM account_allowed_agents WHERE account = ?1")
            .bind(name)
            .execute(&mut *tx)
            .await?;
        let rows = sqlx::query("DELETE FROM accounts WHERE name = ?1")
            .bind(name)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if rows == 0 {
            anyhow::bail!("account '{}' not found", name);
        }
        tx.commit().await?;
        Ok(())
    }

    // ---- account → agent whitelist ------------------------------------

    pub async fn list_allowed_agents(&self, account: &str) -> Result<Vec<String>> {
        let rows = sqlx::query(
            "SELECT agent FROM account_allowed_agents WHERE account = ?1 ORDER BY agent",
        )
        .bind(account)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| r.get("agent")).collect())
    }

    pub async fn is_agent_allowed(&self, account: &str, agent: &str) -> Result<bool> {
        let row = sqlx::query(
            "SELECT 1 AS one FROM account_allowed_agents
              WHERE account = ?1 AND agent = ?2",
        )
        .bind(account)
        .bind(agent)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// Replace this account's allowlist with the given set atomically.
    /// Caller is responsible for whatever de-duplication / validation
    /// makes sense (admin UI). An empty `agents` slice clears the list,
    /// which under strict-whitelist semantics means "this account can
    /// connect to nothing" — useful for soft-disable.
    pub async fn set_allowed_agents(&self, account: &str, agents: &[String]) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM account_allowed_agents WHERE account = ?1")
            .bind(account)
            .execute(&mut *tx)
            .await?;
        for agent in agents {
            sqlx::query(
                "INSERT OR IGNORE INTO account_allowed_agents (account, agent) VALUES (?1, ?2)",
            )
            .bind(account)
            .bind(agent)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Distinct agent names that still appear in the ACL table. The
    /// admin layer unions this with `registry.list_active()` to build
    /// the "known agents" picker. Sessions history is intentionally
    /// NOT included — once an admin has cleared an old agent from
    /// the ACL it should stop showing up, even though sessions rows
    /// (audit history) still reference its old name.
    pub async fn distinct_known_agents(&self) -> Result<Vec<String>> {
        let rows = sqlx::query(
            "SELECT DISTINCT agent FROM account_allowed_agents ORDER BY agent",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| r.get("agent")).collect())
    }

    /// Wipe every ACL row that names this agent. Used by the admin
    /// UI's "delete agent" action when an agent name is retired
    /// (renamed, decommissioned, etc).
    pub async fn delete_agent_acl(&self, agent: &str) -> Result<u64> {
        let r = sqlx::query("DELETE FROM account_allowed_agents WHERE agent = ?1")
            .bind(agent)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }

    pub async fn list_allowed_accounts_for_agent(&self, agent: &str) -> Result<Vec<String>> {
        let rows = sqlx::query(
            "SELECT account FROM account_allowed_agents WHERE agent = ?1 ORDER BY account",
        )
        .bind(agent)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| r.get("account")).collect())
    }

    pub async fn set_allowed_accounts_for_agent(
        &self,
        agent: &str,
        accounts: &[String],
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM account_allowed_agents WHERE agent = ?1")
            .bind(agent)
            .execute(&mut *tx)
            .await?;
        for account in accounts {
            sqlx::query(
                "INSERT OR IGNORE INTO account_allowed_agents (account, agent) VALUES (?1, ?2)",
            )
            .bind(account)
            .bind(agent)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn count_allowed_accounts_per_agent(&self) -> Result<Vec<(String, i64)>> {
        let rows = sqlx::query(
            "SELECT agent, COUNT(*) AS n FROM account_allowed_agents GROUP BY agent",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| (r.get::<String, _>("agent"), r.get::<i64, _>("n")))
            .collect())
    }

    // ---- audit ---------------------------------------------------------

    pub async fn list_audit_events(
        &self,
        f: &AuditFilter,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<AuditDisplayRow>> {
        use sqlx::QueryBuilder;
        let mut qb = QueryBuilder::new(
            "SELECT id, ts, kind, account, agent, session_id, workspace, detail
               FROM audit_events
              WHERE 1=1",
        );
        if let Some(v) = &f.account {
            qb.push(" AND account = ").push_bind(v.clone());
        }
        if let Some(v) = &f.agent {
            qb.push(" AND agent = ").push_bind(v.clone());
        }
        if let Some(v) = &f.kind {
            qb.push(" AND kind = ").push_bind(v.clone());
        }
        if let Some(v) = f.since {
            qb.push(" AND ts >= ").push_bind(v);
        }
        if let Some(v) = f.until {
            qb.push(" AND ts <= ").push_bind(v);
        }
        qb.push(" ORDER BY ts DESC, id DESC LIMIT ")
            .push_bind(limit)
            .push(" OFFSET ")
            .push_bind(offset);
        let rows = qb.build().fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|r| AuditDisplayRow {
                id: r.get("id"),
                ts: r.get("ts"),
                kind: r.get("kind"),
                account: r.get("account"),
                agent: r.get("agent"),
                session_id: r.get("session_id"),
                workspace: r.get("workspace"),
                detail: r.get("detail"),
            })
            .collect())
    }

    pub async fn count_audit_events(&self, f: &AuditFilter) -> Result<i64> {
        use sqlx::QueryBuilder;
        let mut qb = QueryBuilder::new("SELECT COUNT(*) AS n FROM audit_events WHERE 1=1");
        if let Some(v) = &f.account {
            qb.push(" AND account = ").push_bind(v.clone());
        }
        if let Some(v) = &f.agent {
            qb.push(" AND agent = ").push_bind(v.clone());
        }
        if let Some(v) = &f.kind {
            qb.push(" AND kind = ").push_bind(v.clone());
        }
        if let Some(v) = f.since {
            qb.push(" AND ts >= ").push_bind(v);
        }
        if let Some(v) = f.until {
            qb.push(" AND ts <= ").push_bind(v);
        }
        let row = qb.build().fetch_one(&self.pool).await?;
        Ok(row.get::<i64, _>("n"))
    }

    pub async fn distinct_audit_kinds(&self) -> Result<Vec<String>> {
        let rows = sqlx::query(
            "SELECT DISTINCT kind FROM audit_events ORDER BY kind",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| r.get("kind")).collect())
    }

    /// Best-effort insert; logs at debug on failure so a flaky disk
    /// doesn't break PTY flow.
    pub async fn insert_audit(&self, row: &AuditRow) {
        let res = sqlx::query(
            "INSERT INTO audit_events
                (ts, kind, account, agent, session_id, workspace, detail)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .bind(row.ts)
        .bind(&row.kind)
        .bind(&row.account)
        .bind(&row.agent)
        .bind(&row.session_id)
        .bind(&row.workspace)
        .bind(&row.detail)
        .execute(&self.pool)
        .await;
        if let Err(e) = res {
            tracing::debug!(error = %e, "audit insert failed");
        }
    }

    // ---- sessions ------------------------------------------------------

    pub async fn start_session(
        &self,
        session_id: &str,
        account: &str,
        agent: &str,
        workspace: &str,
    ) {
        let res = sqlx::query(
            "INSERT OR REPLACE INTO sessions
                (session_id, account, agent, workspace, started_at, ended_at, ended_reason)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL)",
        )
        .bind(session_id)
        .bind(account)
        .bind(agent)
        .bind(workspace)
        .bind(chrono::Utc::now().timestamp())
        .execute(&self.pool)
        .await;
        if let Err(e) = res {
            tracing::debug!(error = %e, "session start insert failed");
        }
    }

    pub async fn list_sessions(
        &self,
        f: &SessionsFilter,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<SessionRow>> {
        use sqlx::QueryBuilder;
        let mut qb = QueryBuilder::new(
            "SELECT session_id, account, agent, workspace, started_at, ended_at, ended_reason
               FROM sessions WHERE 1=1",
        );
        if let Some(v) = &f.account {
            qb.push(" AND account = ").push_bind(v.clone());
        }
        if let Some(v) = &f.agent {
            qb.push(" AND agent = ").push_bind(v.clone());
        }
        if let Some(v) = &f.workspace {
            qb.push(" AND workspace = ").push_bind(v.clone());
        }
        if f.active_only {
            qb.push(" AND ended_at IS NULL");
        }
        if let Some(v) = f.since {
            qb.push(" AND started_at >= ").push_bind(v);
        }
        qb.push(" ORDER BY started_at DESC LIMIT ")
            .push_bind(limit)
            .push(" OFFSET ")
            .push_bind(offset);
        let rows = qb.build().fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|r| SessionRow {
                session_id: r.get("session_id"),
                account: r.get("account"),
                agent: r.get("agent"),
                workspace: r.get("workspace"),
                started_at: r.get("started_at"),
                ended_at: r.get("ended_at"),
                ended_reason: r.get("ended_reason"),
            })
            .collect())
    }

    pub async fn count_sessions(&self, f: &SessionsFilter) -> Result<i64> {
        use sqlx::QueryBuilder;
        let mut qb = QueryBuilder::new("SELECT COUNT(*) AS n FROM sessions WHERE 1=1");
        if let Some(v) = &f.account {
            qb.push(" AND account = ").push_bind(v.clone());
        }
        if let Some(v) = &f.agent {
            qb.push(" AND agent = ").push_bind(v.clone());
        }
        if let Some(v) = &f.workspace {
            qb.push(" AND workspace = ").push_bind(v.clone());
        }
        if f.active_only {
            qb.push(" AND ended_at IS NULL");
        }
        if let Some(v) = f.since {
            qb.push(" AND started_at >= ").push_bind(v);
        }
        let row = qb.build().fetch_one(&self.pool).await?;
        Ok(row.get::<i64, _>("n"))
    }

    pub async fn get_session(&self, session_id: &str) -> Result<Option<SessionRow>> {
        let row = sqlx::query(
            "SELECT session_id, account, agent, workspace, started_at, ended_at, ended_reason
               FROM sessions WHERE session_id = ?1",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| SessionRow {
            session_id: r.get("session_id"),
            account: r.get("account"),
            agent: r.get("agent"),
            workspace: r.get("workspace"),
            started_at: r.get("started_at"),
            ended_at: r.get("ended_at"),
            ended_reason: r.get("ended_reason"),
        }))
    }

    /// On hub startup any session row still flagged "live" (ended_at
    /// IS NULL) is an orphan from the previous hub process — nothing
    /// is actually attached to it any more. Close them all out so the
    /// admin dashboard tells the truth.
    pub async fn close_orphan_sessions(&self, reason: &str) -> Result<u64> {
        let r = sqlx::query(
            "UPDATE sessions SET ended_at = ?1, ended_reason = ?2 WHERE ended_at IS NULL",
        )
        .bind(chrono::Utc::now().timestamp())
        .bind(reason)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected())
    }

    /// Currently-active sessions (no end recorded). Quick stats card.
    pub async fn count_active_sessions(&self) -> Result<i64> {
        let row = sqlx::query("SELECT COUNT(*) AS n FROM sessions WHERE ended_at IS NULL")
            .fetch_one(&self.pool)
            .await?;
        Ok(row.get::<i64, _>("n"))
    }

    // ---- messages ------------------------------------------------------

    /// Append one conversation message. Idempotency is the caller's job
    /// (agent dedupes via jsonl_progress offsets).
    pub async fn insert_message(&self, row: &MessageRow) {
        let res = sqlx::query(
            "INSERT INTO messages (cc_session_id, claude_session_id, ts, kind, body)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(&row.cc_session_id)
        .bind(&row.claude_session_id)
        .bind(row.ts)
        .bind(&row.kind)
        .bind(&row.body)
        .execute(&self.pool)
        .await;
        if let Err(e) = res {
            tracing::debug!(error = %e, "message insert failed");
        }
    }

    pub async fn list_messages_for_session(
        &self,
        cc_session_id: &str,
        limit: i64,
    ) -> Result<Vec<MessageDisplayRow>> {
        let rows = sqlx::query(
            "SELECT id, cc_session_id, claude_session_id, ts, kind, body
               FROM messages
              WHERE cc_session_id = ?1
              ORDER BY id ASC
              LIMIT ?2",
        )
        .bind(cc_session_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| MessageDisplayRow {
                id: r.get("id"),
                cc_session_id: r.get("cc_session_id"),
                claude_session_id: r.get("claude_session_id"),
                ts: r.get("ts"),
                kind: r.get("kind"),
                body: r.get("body"),
            })
            .collect())
    }

    pub async fn count_messages_for_session(&self, cc_session_id: &str) -> Result<i64> {
        let row =
            sqlx::query("SELECT COUNT(*) AS n FROM messages WHERE cc_session_id = ?1")
                .bind(cc_session_id)
                .fetch_one(&self.pool)
                .await?;
        Ok(row.get::<i64, _>("n"))
    }

    /// Number of sessions started within the last `seconds` seconds.
    pub async fn count_sessions_since(&self, seconds: i64) -> Result<i64> {
        let cutoff = chrono::Utc::now().timestamp() - seconds;
        let row = sqlx::query("SELECT COUNT(*) AS n FROM sessions WHERE started_at >= ?1")
            .bind(cutoff)
            .fetch_one(&self.pool)
            .await?;
        Ok(row.get::<i64, _>("n"))
    }

    /// Latest `started_at` per (agent, account, workspace). Used by the
    /// admin Workspaces page to show when each slot was last touched.
    pub async fn last_started_per_workspace(
        &self,
    ) -> Result<Vec<(String, String, String, i64)>> {
        let rows = sqlx::query(
            "SELECT agent, account, workspace, MAX(started_at) AS last_started
               FROM sessions
              GROUP BY agent, account, workspace",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                (
                    r.get::<String, _>("agent"),
                    r.get::<String, _>("account"),
                    r.get::<String, _>("workspace"),
                    r.get::<i64, _>("last_started"),
                )
            })
            .collect())
    }

    pub async fn end_session(&self, session_id: &str, reason: Option<&str>) {
        let res = sqlx::query(
            "UPDATE sessions
                SET ended_at = ?1, ended_reason = ?2
              WHERE session_id = ?3 AND ended_at IS NULL",
        )
        .bind(chrono::Utc::now().timestamp())
        .bind(reason)
        .bind(session_id)
        .execute(&self.pool)
        .await;
        if let Err(e) = res {
            tracing::debug!(error = %e, "session end update failed");
        }
    }

    // ---- sessions (admin + webterm; survive hub restart) --------------

    pub async fn insert_admin_session(&self, sid: &str, expires_at: i64) -> Result<()> {
        sqlx::query(
            "INSERT INTO admin_sessions (sid, expires_at) VALUES (?1, ?2)",
        )
        .bind(sid)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn valid_admin_session(&self, sid: &str) -> Result<bool> {
        let now = chrono::Utc::now().timestamp();
        let row = sqlx::query(
            "SELECT 1 FROM admin_sessions WHERE sid = ?1 AND expires_at > ?2",
        )
        .bind(sid)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    pub async fn delete_admin_session(&self, sid: &str) -> Result<()> {
        sqlx::query("DELETE FROM admin_sessions WHERE sid = ?1")
            .bind(sid)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn insert_user_session(
        &self,
        sid: &str,
        account: &str,
        expires_at: i64,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO user_sessions (sid, account, expires_at) VALUES (?1, ?2, ?3)",
        )
        .bind(sid)
        .bind(account)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn user_session_account(&self, sid: &str) -> Result<Option<String>> {
        let now = chrono::Utc::now().timestamp();
        let row = sqlx::query(
            "SELECT account FROM user_sessions WHERE sid = ?1 AND expires_at > ?2",
        )
        .bind(sid)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| r.get::<String, _>("account")))
    }

    pub async fn delete_user_session(&self, sid: &str) -> Result<()> {
        sqlx::query("DELETE FROM user_sessions WHERE sid = ?1")
            .bind(sid)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Drop sessions whose expiry has elapsed. Called once on hub
    /// startup so the tables don't grow unboundedly across long
    /// uptimes / many cookie cycles.
    pub async fn cleanup_expired_sessions(&self) -> Result<u64> {
        let now = chrono::Utc::now().timestamp();
        let a = sqlx::query("DELETE FROM admin_sessions WHERE expires_at <= ?1")
            .bind(now)
            .execute(&self.pool)
            .await?
            .rows_affected();
        let u = sqlx::query("DELETE FROM user_sessions WHERE expires_at <= ?1")
            .bind(now)
            .execute(&self.pool)
            .await?
            .rows_affected();
        Ok(a + u)
    }

    // ---- user preferences (opaque JSON blob per account) --------------

    /// Returns the raw JSON blob the webterm last stored for this
    /// account, or `None` if the account has never saved preferences.
    /// The hub does not parse `data` — webterm owns the schema.
    pub async fn get_user_preferences(&self, account: &str) -> Result<Option<String>> {
        let row = sqlx::query("SELECT data FROM user_preferences WHERE account = ?1")
            .bind(account)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.get::<String, _>("data")))
    }

    /// Upsert the per-account preferences blob. `data` must be valid
    /// JSON; this method does not validate (the API handler does).
    pub async fn set_user_preferences(&self, account: &str, data: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO user_preferences (account, data, updated_at)
                  VALUES (?1, ?2, ?3)
             ON CONFLICT(account) DO UPDATE
                SET data = excluded.data,
                    updated_at = excluded.updated_at",
        )
        .bind(account)
        .bind(data)
        .bind(chrono::Utc::now().timestamp())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ---- workspaces (v1.13 hub-managed workspace metadata) ------------

    /// Every workspace owned by `account`, alphabetised. Returns an
    /// empty Vec for accounts that have never created a workspace.
    pub async fn list_workspaces(&self, account: &str) -> Result<Vec<WorkspaceRow>> {
        let rows = sqlx::query(
            "SELECT account, name, created_at, locked_by_agent, locked_at,
                    last_sync_at, size_bytes
               FROM workspaces
              WHERE account = ?1
              ORDER BY name",
        )
        .bind(account)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| WorkspaceRow {
                account: r.get("account"),
                name: r.get("name"),
                created_at: r.get("created_at"),
                locked_by_agent: r.get("locked_by_agent"),
                locked_at: r.get("locked_at"),
                last_sync_at: r.get("last_sync_at"),
                size_bytes: r.get("size_bytes"),
            })
            .collect())
    }

    /// Insert a brand-new workspace row. Errors on (account, name)
    /// conflict — callers should treat that as "workspace already
    /// exists, pick another name" rather than silently merging.
    pub async fn create_workspace(&self, account: &str, name: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO workspaces (account, name, created_at, size_bytes)
                  VALUES (?1, ?2, ?3, 0)",
        )
        .bind(account)
        .bind(name)
        .bind(chrono::Utc::now().timestamp())
        .execute(&self.pool)
        .await
        .with_context(|| format!("creating workspace {account}/{name}"))?;
        Ok(())
    }

    /// Drop the workspace metadata row. The caller is responsible for
    /// deleting the on-disk copy via `WorkspaceStorage::delete`; this
    /// method only touches the DB.
    pub async fn delete_workspace(&self, account: &str, name: &str) -> Result<()> {
        sqlx::query("DELETE FROM workspaces WHERE account = ?1 AND name = ?2")
            .bind(account)
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Set or clear the workspace lock. Passing `Some(agent)` records
    /// the agent and the current timestamp; passing `None` clears both
    /// columns. Errors if the workspace row does not exist.
    pub async fn set_workspace_lock(
        &self,
        account: &str,
        name: &str,
        agent: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let rows = match agent {
            Some(a) => sqlx::query(
                "UPDATE workspaces
                    SET locked_by_agent = ?1, locked_at = ?2
                  WHERE account = ?3 AND name = ?4",
            )
            .bind(a)
            .bind(now)
            .bind(account)
            .bind(name)
            .execute(&self.pool)
            .await?
            .rows_affected(),
            None => sqlx::query(
                "UPDATE workspaces
                    SET locked_by_agent = NULL, locked_at = NULL
                  WHERE account = ?1 AND name = ?2",
            )
            .bind(account)
            .bind(name)
            .execute(&self.pool)
            .await?
            .rows_affected(),
        };
        if rows == 0 {
            anyhow::bail!("workspace '{}/{}' not found", account, name);
        }
        Ok(())
    }

    /// Returns the agent name currently holding the lock, or `None` if
    /// the workspace is unlocked or doesn't exist. Treating "missing
    /// workspace" and "unlocked workspace" identically keeps callers
    /// simple — both states mean "no agent owns this".
    pub async fn get_workspace_lock(
        &self,
        account: &str,
        name: &str,
    ) -> Result<Option<String>> {
        let row = sqlx::query(
            "SELECT locked_by_agent FROM workspaces
              WHERE account = ?1 AND name = ?2",
        )
        .bind(account)
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.and_then(|r| r.get::<Option<String>, _>("locked_by_agent")))
    }

    /// Record a successful sync from an agent: stamp `last_sync_at` to
    /// now and store the new aggregate size. Errors if the workspace
    /// row is missing — sync should never produce metadata for an
    /// uncreated workspace.
    pub async fn update_workspace_sync_meta(
        &self,
        account: &str,
        name: &str,
        size_bytes: i64,
    ) -> Result<()> {
        let rows = sqlx::query(
            "UPDATE workspaces
                SET last_sync_at = ?1, size_bytes = ?2
              WHERE account = ?3 AND name = ?4",
        )
        .bind(chrono::Utc::now().timestamp())
        .bind(size_bytes)
        .bind(account)
        .bind(name)
        .execute(&self.pool)
        .await?
        .rows_affected();
        if rows == 0 {
            anyhow::bail!("workspace '{}/{}' not found", account, name);
        }
        Ok(())
    }

    /// Clear `locked_by_agent` on every workspace currently held by
    /// `agent`. Called when the agent disconnects so a workspace
    /// doesn't stay "owned" by a dead connection forever. Returns the
    /// number of workspaces unlocked (useful for tracing).
    pub async fn release_all_workspace_locks_for_agent(&self, agent: &str) -> Result<u64> {
        let r = sqlx::query(
            "UPDATE workspaces
                SET locked_by_agent = NULL, locked_at = NULL
              WHERE locked_by_agent = ?1",
        )
        .bind(agent)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected())
    }

    /// Every workspace across every account. Powers the admin
    /// inventory page in v1.13; no pagination yet (volumes are tiny
    /// relative to audit / sessions).
    pub async fn list_all_workspaces(&self) -> Result<Vec<WorkspaceRow>> {
        let rows = sqlx::query(
            "SELECT account, name, created_at, locked_by_agent, locked_at,
                    last_sync_at, size_bytes
               FROM workspaces
              ORDER BY account, name",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| WorkspaceRow {
                account: r.get("account"),
                name: r.get("name"),
                created_at: r.get("created_at"),
                locked_by_agent: r.get("locked_by_agent"),
                locked_at: r.get("locked_at"),
                last_sync_at: r.get("last_sync_at"),
                size_bytes: r.get("size_bytes"),
            })
            .collect())
    }

    /// Queue a "the hub force-took your lock, drop your local copy"
    /// instruction for `agent`. Idempotent via `INSERT OR REPLACE` —
    /// re-queueing the same triple just refreshes `queued_at`.
    pub async fn queue_pending_cleanup(
        &self,
        agent: &str,
        account: &str,
        workspace: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT OR REPLACE INTO pending_workspace_cleanups
                (agent, account, workspace, queued_at)
             VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(agent)
        .bind(account)
        .bind(workspace)
        .bind(chrono::Utc::now().timestamp())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Drain the cleanup queue for `agent`: return every `(account,
    /// workspace)` pair the agent should rm -rf, and delete those rows
    /// in the same transaction so a second call returns nothing. The
    /// agent reports back per-pair success out-of-band; if it crashes
    /// mid-cleanup we just re-queue.
    pub async fn take_pending_cleanups(
        &self,
        agent: &str,
    ) -> Result<Vec<(String, String)>> {
        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query(
            "SELECT account, workspace FROM pending_workspace_cleanups
              WHERE agent = ?1
              ORDER BY queued_at",
        )
        .bind(agent)
        .fetch_all(&mut *tx)
        .await?;
        let out: Vec<(String, String)> = rows
            .into_iter()
            .map(|r| (r.get::<String, _>("account"), r.get::<String, _>("workspace")))
            .collect();
        sqlx::query("DELETE FROM pending_workspace_cleanups WHERE agent = ?1")
            .bind(agent)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(out)
    }
}

#[derive(Debug, Clone, Default)]
pub struct AuditRow {
    pub ts: i64,
    pub kind: String,
    pub account: Option<String>,
    pub agent: Option<String>,
    pub session_id: Option<String>,
    pub workspace: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AuditDisplayRow {
    pub id: i64,
    pub ts: i64,
    pub kind: String,
    pub account: Option<String>,
    pub agent: Option<String>,
    pub session_id: Option<String>,
    pub workspace: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SessionRow {
    pub session_id: String,
    pub account: String,
    pub agent: String,
    pub workspace: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub ended_reason: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SessionsFilter {
    pub account: Option<String>,
    pub agent: Option<String>,
    pub workspace: Option<String>,
    pub active_only: bool,
    pub since: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct MessageRow {
    pub cc_session_id: String,
    pub claude_session_id: String,
    pub ts: i64,
    pub kind: String,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct MessageDisplayRow {
    pub id: i64,
    pub cc_session_id: String,
    pub claude_session_id: String,
    pub ts: i64,
    pub kind: String,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct WorkspaceRow {
    pub account: String,
    pub name: String,
    pub created_at: i64,
    /// Agent that currently holds the write lock, if any.
    pub locked_by_agent: Option<String>,
    /// Timestamp the lock was last acquired. NULL iff `locked_by_agent`
    /// is also NULL.
    pub locked_at: Option<i64>,
    /// Last successful push from an agent. NULL until the workspace
    /// has been synced at least once.
    pub last_sync_at: Option<i64>,
    pub size_bytes: i64,
}

#[derive(Debug, Clone, Default)]
pub struct AuditFilter {
    pub account: Option<String>,
    pub agent: Option<String>,
    pub kind: Option<String>,
    pub since: Option<i64>,
    pub until: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spin up a fresh on-disk SQLite under the OS temp dir. Each
    /// test gets a unique file so they can run in parallel without
    /// stepping on each other.
    async fn fresh_db() -> Db {
        let path = std::env::temp_dir().join(format!(
            "cloudcode-hub-db-test-{}.sqlite",
            uuid::Uuid::new_v4().simple()
        ));
        Db::open(&path).await.expect("open db")
    }

    /// `list_workspaces` is the load-bearing call behind the new
    /// `ListWorkspaces` handler. Lock + sync metadata MUST come
    /// through unchanged for the client to render the picker.
    #[tokio::test]
    async fn list_workspaces_surfaces_lock_and_sync_metadata() {
        let db = fresh_db().await;
        db.create_workspace("alice", "demo").await.unwrap();
        db.create_workspace("alice", "scratch").await.unwrap();
        // Lock + size for one of them.
        db.set_workspace_lock("alice", "demo", Some("box-1"))
            .await
            .unwrap();
        db.update_workspace_sync_meta("alice", "demo", 4096)
            .await
            .unwrap();

        let rows = db.list_workspaces("alice").await.unwrap();
        assert_eq!(rows.len(), 2);
        // ORDER BY name → demo, scratch
        assert_eq!(rows[0].name, "demo");
        assert_eq!(rows[0].locked_by_agent.as_deref(), Some("box-1"));
        assert_eq!(rows[0].size_bytes, 4096);
        assert!(rows[0].last_sync_at.is_some());
        assert_eq!(rows[1].name, "scratch");
        assert!(rows[1].locked_by_agent.is_none());
        assert_eq!(rows[1].size_bytes, 0);

        // Another account sees nothing.
        assert!(db.list_workspaces("bob").await.unwrap().is_empty());
    }

    /// Lock takeover flow:
    ///   - agent A acquires the lock.
    ///   - hub takes it over for agent B: queues a cleanup row for A,
    ///     sets the lock to B.
    ///   - on A's next "hello", drain returns its cleanup row exactly
    ///     once. A second drain on A returns empty.
    #[tokio::test]
    async fn workspace_lock_force_takeover_with_pending_cleanup() {
        let db = fresh_db().await;
        db.create_workspace("alice", "ws").await.unwrap();

        // A acquires.
        db.set_workspace_lock("alice", "ws", Some("agent-A"))
            .await
            .unwrap();
        assert_eq!(
            db.get_workspace_lock("alice", "ws").await.unwrap(),
            Some("agent-A".to_string())
        );

        // Hub force-takes for B: queue cleanup for A, swap the lock.
        db.queue_pending_cleanup("agent-A", "alice", "ws")
            .await
            .unwrap();
        db.set_workspace_lock("alice", "ws", Some("agent-B"))
            .await
            .unwrap();
        assert_eq!(
            db.get_workspace_lock("alice", "ws").await.unwrap(),
            Some("agent-B".to_string())
        );

        // A reconnects: drain its cleanups.
        let pending = db.take_pending_cleanups("agent-A").await.unwrap();
        assert_eq!(pending, vec![("alice".to_string(), "ws".to_string())]);
        // Drain is exhaustive — a second call after the agent has
        // applied the cleanup returns nothing.
        assert!(db.take_pending_cleanups("agent-A").await.unwrap().is_empty());
    }

    /// Disconnect path: every workspace this agent held has to be
    /// freed so the next OpenSession can proceed without `force=true`.
    #[tokio::test]
    async fn release_all_locks_clears_only_target_agents_rows() {
        let db = fresh_db().await;
        db.create_workspace("alice", "a").await.unwrap();
        db.create_workspace("alice", "b").await.unwrap();
        db.create_workspace("alice", "c").await.unwrap();
        db.set_workspace_lock("alice", "a", Some("agent-1"))
            .await
            .unwrap();
        db.set_workspace_lock("alice", "b", Some("agent-1"))
            .await
            .unwrap();
        db.set_workspace_lock("alice", "c", Some("agent-2"))
            .await
            .unwrap();

        let freed = db
            .release_all_workspace_locks_for_agent("agent-1")
            .await
            .unwrap();
        assert_eq!(freed, 2);

        assert!(db.get_workspace_lock("alice", "a").await.unwrap().is_none());
        assert!(db.get_workspace_lock("alice", "b").await.unwrap().is_none());
        assert_eq!(
            db.get_workspace_lock("alice", "c").await.unwrap(),
            Some("agent-2".to_string())
        );
    }

    /// Admin-facing `list_all_workspaces` collapses across accounts
    /// and orders deterministically (account, name).
    #[tokio::test]
    async fn list_all_workspaces_orders_across_accounts() {
        let db = fresh_db().await;
        db.create_workspace("bob", "x").await.unwrap();
        db.create_workspace("alice", "z").await.unwrap();
        db.create_workspace("alice", "y").await.unwrap();
        let rows = db.list_all_workspaces().await.unwrap();
        let names: Vec<(String, String)> = rows
            .into_iter()
            .map(|r| (r.account, r.name))
            .collect();
        assert_eq!(
            names,
            vec![
                ("alice".to_string(), "y".to_string()),
                ("alice".to_string(), "z".to_string()),
                ("bob".to_string(), "x".to_string()),
            ]
        );
    }
}
