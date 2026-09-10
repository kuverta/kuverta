//! Schema and forward-only migrations, tracked via `PRAGMA user_version`.
//!
//! Migrations are append-only: never edit a shipped entry, add a new one.

use rusqlite::Connection;

use crate::Result;

/// Each entry is applied in order; the index + 1 becomes `user_version`.
const MIGRATIONS: &[&str] = &[
    // v1 — accounts, folders, messages, locations, search, classifier output.
    r#"
CREATE TABLE account (
    id            INTEGER PRIMARY KEY,
    label         TEXT    NOT NULL,
    email         TEXT    NOT NULL UNIQUE,
    imap_host     TEXT    NOT NULL,
    imap_port     INTEGER NOT NULL,
    imap_security TEXT    NOT NULL,
    username      TEXT    NOT NULL,
    auth_method   TEXT    NOT NULL,
    created_at    INTEGER NOT NULL
);

CREATE TABLE folder (
    id             INTEGER PRIMARY KEY,
    account_id     INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    name           TEXT    NOT NULL,
    special_use    TEXT,
    -- IMAP sync state. A changed uid_validity invalidates every cached UID for
    -- this folder and forces a full resync.
    uid_validity   INTEGER,
    uid_next       INTEGER,
    highest_modseq INTEGER,
    last_synced_at INTEGER,
    UNIQUE (account_id, name)
);

CREATE TABLE message (
    id                INTEGER PRIMARY KEY,
    account_id        INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    -- Identity within the account: "mid:<message-id>" or "synth:<sha256>".
    -- See dedup.rs. The UNIQUE constraint is what collapses Gmail's
    -- labels-as-folders duplicates into a single row.
    dedup_key         TEXT    NOT NULL,
    rfc822_message_id TEXT,
    subject           TEXT,
    from_name         TEXT,
    from_addr         TEXT,
    date_utc          INTEGER,
    size_bytes        INTEGER,
    snippet           TEXT,
    has_attachments   INTEGER NOT NULL DEFAULT 0,
    list_id           TEXT,
    in_reply_to       TEXT,
    body_path         TEXT,
    first_seen_at     INTEGER NOT NULL,
    UNIQUE (account_id, dedup_key)
);

CREATE INDEX message_by_date ON message (account_id, date_utc DESC);
CREATE INDEX message_by_sender ON message (account_id, from_addr);
CREATE INDEX message_by_list ON message (account_id, list_id);

-- One message, many folders. The primary key is (folder, uid) because that is
-- what the server guarantees unique; message_id is the many side.
CREATE TABLE message_location (
    message_id INTEGER NOT NULL REFERENCES message(id) ON DELETE CASCADE,
    folder_id  INTEGER NOT NULL REFERENCES folder(id) ON DELETE CASCADE,
    uid        INTEGER NOT NULL,
    flags      TEXT    NOT NULL DEFAULT '',
    PRIMARY KEY (folder_id, uid)
);

CREATE INDEX location_by_message ON message_location (message_id);

-- Standalone (not external-content) FTS5: the searchable body text is not a
-- column on `message`, so external content would not buy anything.
CREATE VIRTUAL TABLE message_fts USING fts5(subject, sender, body);

CREATE TRIGGER message_fts_delete AFTER DELETE ON message BEGIN
    DELETE FROM message_fts WHERE rowid = old.id;
END;

-- Rules and model verdicts coexist deliberately: running both and recording
-- where they disagree is the only way to tell whether the model earns its
-- latency. See docs/implementation-plan.md section 4.
CREATE TABLE classification (
    id         INTEGER PRIMARY KEY,
    message_id INTEGER NOT NULL REFERENCES message(id) ON DELETE CASCADE,
    category   TEXT    NOT NULL,
    confidence REAL,
    source     TEXT    NOT NULL,
    model      TEXT,
    latency_ms INTEGER,
    created_at INTEGER NOT NULL
);

CREATE INDEX classification_by_message ON classification (message_id, source);

-- Every correction is training data. Captured from the first commit even
-- though nothing consumes it until the model layer lands.
CREATE TABLE correction (
    id            INTEGER PRIMARY KEY,
    message_id    INTEGER NOT NULL REFERENCES message(id) ON DELETE CASCADE,
    from_category TEXT,
    to_category   TEXT    NOT NULL,
    created_at    INTEGER NOT NULL
);
"#,
    // v2 — OAuth2 app registration, for accounts that cannot use a password.
    // No secret is stored here: the device flow is a public-client grant, and
    // the refresh token lives in the OS keychain.
    r#"
ALTER TABLE account ADD COLUMN oauth_client_id TEXT;
ALTER TABLE account ADD COLUMN oauth_tenant TEXT;
"#,
    // v3 — SMTP submission endpoint, for accounts that can send.
    //
    // Nullable as a group: an account registered before send existed has no
    // SMTP endpoint and stays receive-only until one is configured. There is
    // no separate credential — submission reuses the account's `AuthProvider`,
    // so nothing secret lands here either.
    r#"
ALTER TABLE account ADD COLUMN smtp_host TEXT;
ALTER TABLE account ADD COLUMN smtp_port INTEGER;
ALTER TABLE account ADD COLUMN smtp_security TEXT;
"#,
];

pub(crate) fn migrate(conn: &Connection) -> Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let current = current as usize;

    if current > MIGRATIONS.len() {
        return Err(crate::StoreError::SchemaTooNew {
            found: current,
            supported: MIGRATIONS.len(),
        });
    }

    for (index, sql) in MIGRATIONS.iter().enumerate().skip(current) {
        let version = index + 1;
        tracing::debug!(version, "applying migration");
        conn.execute_batch(sql)?;
        // PRAGMA does not accept bound parameters.
        conn.execute_batch(&format!("PRAGMA user_version = {version}"))?;
    }

    Ok(())
}
