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
    // v4 — the pending-operation queue for mailbox mutations (plan section 1a,
    // stage 2).
    //
    // Intent is recorded here *before* the server is touched, never after. A
    // crash between the user acting and the round trip completing must leave
    // the intent durable rather than lose it, and an operation that may or may
    // not have reached the server must be re-checkable rather than blindly
    // retried.
    //
    // The source columns exist for exactly that re-check. A UID means nothing
    // without the UIDVALIDITY it was issued under, and neither proves the UID
    // still refers to the mail the user acted on — so the Message-ID they
    // expected is recorded too, and the executor verifies all three before it
    // mutates anything.
    r#"
CREATE TABLE operation (
    id                  INTEGER PRIMARY KEY,
    account_id          INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    -- Cascades: if sync finds the message gone from the server entirely, a
    -- queued operation on it is moot rather than pending.
    message_id          INTEGER NOT NULL REFERENCES message(id) ON DELETE CASCADE,
    kind                TEXT    NOT NULL,

    source_folder_id    INTEGER NOT NULL REFERENCES folder(id) ON DELETE CASCADE,
    source_uid          INTEGER NOT NULL,
    source_uid_validity INTEGER,
    expect_message_id   TEXT,

    -- kind = 'move'. A folder name rather than an id: the destination need not
    -- have been synced yet, and names are what IMAP commands take anyway.
    target_folder       TEXT,

    -- kind = 'flag'
    flag                TEXT,
    flag_set            INTEGER,

    state               TEXT    NOT NULL,
    -- The undo window: not eligible to be sent to the server before this.
    -- Cancelling is allowed at any point while the operation is still pending,
    -- so this is a floor on the grace period, not the whole of it.
    execute_after       INTEGER NOT NULL,
    attempts            INTEGER NOT NULL DEFAULT 0,
    last_error          TEXT,
    created_at          INTEGER NOT NULL,
    settled_at          INTEGER
);

CREATE INDEX operation_due ON operation (account_id, state, execute_after);
CREATE INDEX operation_by_message ON operation (message_id, state);
"#,
    // v5 — folders a sync should leave alone.
    //
    // Gmail is the reason. Its `[Gmail]/All Mail` holds a copy of every
    // message, so a mailbox whose mail averages two labels is fetched twice
    // over: deduplication keeps one row and one body, but the bytes still
    // cross the wire. Skipping it is the difference between downloading a
    // mailbox once and downloading it twice.
    //
    // Stored per account rather than per folder because it has to be settable
    // *before* the first sync — which is exactly when it matters, and before
    // any folder row exists to hang a flag on. `fuckmail check` prints the
    // names without syncing anything.
    r#"
CREATE TABLE folder_exclusion (
    account_id INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    -- A folder name, or an RFC 6154 attribute like \All. The attribute form
    -- survives a provider renaming its folders, and says what is meant.
    pattern    TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (account_id, pattern)
);
"#,
    // v6 — which OAuth2 provider an account authorises against.
    //
    // Not cosmetic: the grant differs. Microsoft uses the device flow, and
    // Google cannot — it does not issue `https://mail.google.com/` to the
    // limited-input device grant at all, so Gmail needs the loopback redirect
    // instead. Inferring this from the hostname would work until somebody used
    // a custom domain in front of Google.
    //
    // Still no secret here. Google issues a client secret even for desktop
    // clients, and it goes in the keychain with everything else.
    r#"
ALTER TABLE account ADD COLUMN oauth_provider TEXT;
"#,
    // v7 — postal addresses. A physical address is an account and Paperless is
    // its server, so this is the paper counterpart of `account`: where the
    // instance lives, and which of its documents belong to this address. The
    // API token is not here — it goes in the OS keychain, exactly as an
    // account's password does.
    r#"
CREATE TABLE paper_mailbox (
    id             INTEGER PRIMARY KEY,
    label          TEXT NOT NULL,
    base_url       TEXT NOT NULL,
    -- 'everything' | 'tag' | 'correspondent' | 'storage_path'
    selector_kind  TEXT NOT NULL,
    selector_value TEXT,
    created_at     INTEGER NOT NULL
);

-- One address per instance-and-selector. Two mailboxes pointing at the same
-- documents would show the same post twice in one list.
CREATE UNIQUE INDEX paper_mailbox_target
    ON paper_mailbox (base_url, selector_kind, IFNULL(selector_value, ''));
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
