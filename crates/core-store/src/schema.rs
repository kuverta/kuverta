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
    // any folder row exists to hang a flag on. `kuverta check` prints the
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
    // v8 — corrections to post. `correction` hangs off a message row and a
    // document is not one, so post has its own table. Append-only for the
    // reason §3.4 gives: a correction reversed is two events, not an edit, and
    // the reversals are the most interesting rows.
    r#"
CREATE TABLE paper_correction (
    id            INTEGER PRIMARY KEY,
    mailbox_id    INTEGER NOT NULL REFERENCES paper_mailbox(id) ON DELETE CASCADE,
    -- Paperless's id. Not a foreign key: the document lives in Paperless.
    document_id   INTEGER NOT NULL,
    -- Who sent it, when Paperless knew. What a correction teaches.
    correspondent TEXT,
    from_category TEXT,
    to_category   TEXT NOT NULL,
    created_at    INTEGER NOT NULL
);

CREATE INDEX paper_correction_by_document ON paper_correction (mailbox_id, document_id);
CREATE INDEX paper_correction_by_correspondent ON paper_correction (correspondent);
"#,
    // v9 — what an address's API token is filed under in the keychain. It was
    // the row id, and row ids start at 1 in every store: `.devdata` and the
    // real store would share `paper:1`, and saving a token in one silently
    // replaced the other's. A random key is unique across stores and, being
    // part of the row, survives every edit that the id survived.
    r#"
ALTER TABLE paper_mailbox ADD COLUMN token_key TEXT;
UPDATE paper_mailbox SET token_key = lower(hex(randomblob(16))) WHERE token_key IS NULL;
CREATE UNIQUE INDEX paper_mailbox_token_key ON paper_mailbox (token_key);
"#,
    // v10 — what kuverta knows about post that Paperless does not keep:
    // whether a letter has been read here, and a vision model's transcript of
    // a scan whose OCR text is no use. Keyed by instance and document rather
    // than by postal address, because two addresses on one instance can show
    // the same letter, and reading it in one reads it.
    r#"
CREATE TABLE paper_read (
    base_url    TEXT NOT NULL,
    document_id INTEGER NOT NULL,
    read_at     INTEGER NOT NULL,
    PRIMARY KEY (base_url, document_id)
);

CREATE TABLE paper_transcript (
    base_url    TEXT NOT NULL,
    document_id INTEGER NOT NULL,
    model       TEXT NOT NULL,
    text        TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (base_url, document_id)
);
"#,
    // v11 — where models run, and which model each job uses. The local Ollama
    // is there from the start, as it was before this was configurable; a
    // hosted service is added beside it, its key in the keychain under
    // `key_name`. A job with no row uses its default.
    r#"
CREATE TABLE ai_provider (
    id         INTEGER PRIMARY KEY,
    kind       TEXT NOT NULL CHECK (kind IN ('ollama', 'openai')),
    label      TEXT NOT NULL,
    base_url   TEXT NOT NULL,
    key_name   TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL
);

INSERT INTO ai_provider (id, kind, label, base_url, key_name, created_at)
VALUES (1, 'ollama', 'Ollama on this computer', 'http://127.0.0.1:11434',
        lower(hex(randomblob(16))), CAST(strftime('%s', 'now') AS INTEGER));

CREATE TABLE ai_task (
    task        TEXT PRIMARY KEY,
    provider_id INTEGER NOT NULL REFERENCES ai_provider (id),
    model       TEXT NOT NULL
);
"#,
    // v12 — sending later, smart mailboxes, and cleaning up.
    //
    // The outbox is a ledger of promises: the draft as JSON (core-rpc owns its
    // shape), when it is due, and what happened. Smart mailboxes are saved
    // questions, their rules JSON for the same reason. `unsubscription` records
    // every attempt, because "did I already unsubscribe from these?" is the
    // question the next newsletter from them raises.
    //
    // The message columns are headers the store threw away until now. Mail
    // synced before this migration has them NULL and `headers_read = 0`, and is
    // read back from the stored raw message once — see `hygiene.rs`.
    r#"
ALTER TABLE message ADD COLUMN recipients TEXT;
ALTER TABLE message ADD COLUMN list_unsubscribe TEXT;
ALTER TABLE message ADD COLUMN list_unsubscribe_post TEXT;
ALTER TABLE message ADD COLUMN headers_read INTEGER NOT NULL DEFAULT 0;
CREATE INDEX message_unread_headers ON message (account_id, headers_read);

CREATE TABLE outbox (
    id          INTEGER PRIMARY KEY,
    account_id  INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    draft       TEXT    NOT NULL,
    subject     TEXT    NOT NULL,
    recipients  TEXT    NOT NULL,
    send_at     INTEGER NOT NULL,
    -- 'scheduled' | 'sending' | 'sent' | 'failed' | 'cancelled'
    state       TEXT    NOT NULL,
    attempts    INTEGER NOT NULL DEFAULT 0,
    last_error  TEXT,
    created_at  INTEGER NOT NULL,
    sent_at     INTEGER
);
CREATE INDEX outbox_due ON outbox (state, send_at);

CREATE TABLE smart_mailbox (
    id         INTEGER PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    name       TEXT    NOT NULL,
    match_all  INTEGER NOT NULL DEFAULT 1,
    rules      TEXT    NOT NULL,
    -- 'thunderbird' | 'apple_mail' when imported; NULL when made here.
    source     TEXT,
    created_at INTEGER NOT NULL
);

CREATE TABLE unsubscription (
    id         INTEGER PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    -- A List-Id, else a lowercased From address.
    sender     TEXT    NOT NULL,
    -- 'one_click' | 'mailto' | 'browser'
    method     TEXT    NOT NULL,
    target     TEXT    NOT NULL,
    -- 'done' | 'opened' | 'failed'
    state      TEXT    NOT NULL,
    detail     TEXT,
    created_at INTEGER NOT NULL
);
CREATE INDEX unsubscription_by_sender ON unsubscription (account_id, sender);
"#,
    // v13 — how soon a message needs acting on. One verdict per message, the
    // rules' first and a model's in its place when one is asked: advice that
    // orders the "needs attention" view and moves nothing. `in_reply_to` and
    // the sender are indexed for the questions urgency asks — has this been
    // answered, and has this person been written to.
    r#"
CREATE TABLE urgency (
    message_id INTEGER PRIMARY KEY REFERENCES message(id) ON DELETE CASCADE,
    -- 0 nothing to do, 1 can wait, 2 this week, 3 today
    score      INTEGER NOT NULL,
    reason     TEXT    NOT NULL,
    -- 'reply' | 'pay' | 'attend' | 'decide' | 'read' | 'none'
    action     TEXT,
    -- YYYY-MM-DD
    deadline   TEXT,
    -- 'rules' | 'model'
    source     TEXT    NOT NULL,
    model      TEXT,
    created_at INTEGER NOT NULL
);
CREATE INDEX urgency_by_score ON urgency (score);
CREATE INDEX message_by_reply ON message (account_id, in_reply_to);
"#,
    // v14 — profiles: private, one company, another. Accounts and postal
    // addresses point at one or at none; the window shows one profile at a
    // time, or all. Nothing else changes with them.
    r#"
CREATE TABLE profile (
    id         INTEGER PRIMARY KEY,
    name       TEXT    NOT NULL UNIQUE COLLATE NOCASE,
    position   INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);
ALTER TABLE account ADD COLUMN profile_id INTEGER REFERENCES profile(id) ON DELETE SET NULL;
ALTER TABLE paper_mailbox ADD COLUMN profile_id INTEGER REFERENCES profile(id) ON DELETE SET NULL;
"#,
    // v15 — tasks: standing jobs the assistant does to mail. A task is a smart
    // mailbox's rules and an action (JSON; core-rpc owns its shape).
    // `task_seen` is what each has dealt with, so it deals with it once.
    // `task_proposal` is what a task or the assistant would do and is waiting
    // for a person's yes — for tasks that ask first, and anything that sends.
    r#"
CREATE TABLE task (
    id           INTEGER PRIMARY KEY,
    account_id   INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    name         TEXT    NOT NULL,
    match_all    INTEGER NOT NULL DEFAULT 1,
    rules        TEXT    NOT NULL,
    action       TEXT    NOT NULL,
    review       INTEGER NOT NULL DEFAULT 0,
    enabled      INTEGER NOT NULL DEFAULT 1,
    created_at   INTEGER NOT NULL,
    last_run_at  INTEGER,
    last_summary TEXT
);

CREATE TABLE task_seen (
    task_id    INTEGER NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    message_id INTEGER NOT NULL REFERENCES message(id) ON DELETE CASCADE,
    PRIMARY KEY (task_id, message_id)
);

CREATE TABLE task_proposal (
    id         INTEGER PRIMARY KEY,
    -- NULL for what the assistant proposed in the chat.
    task_id    INTEGER REFERENCES task(id) ON DELETE CASCADE,
    account_id INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    message_id INTEGER REFERENCES message(id) ON DELETE CASCADE,
    -- 'move' | 'archive' | 'trash' | 'mark_read' | 'file' | 'reply'
    kind       TEXT    NOT NULL,
    target     TEXT,
    body       TEXT,
    reason     TEXT,
    -- 'pending' | 'done' | 'rejected' | 'failed'
    state      TEXT    NOT NULL,
    detail     TEXT,
    created_at INTEGER NOT NULL
);
CREATE INDEX task_proposal_pending ON task_proposal (account_id, state);
"#,
    // v16 — which rules decided. A rules verdict made by older rules — or
    // from headers kuverta had not stored yet — is decided again when the
    // rules change; this says which verdicts are behind.
    r#"
ALTER TABLE classification ADD COLUMN rules_version INTEGER NOT NULL DEFAULT 1;
"#,
    // v17 — how far down a folder the first pass has come. A first sync walks
    // a folder newest first, so that this morning's mail is on screen in
    // seconds rather than after the whole history; this is the UID it has
    // reached, and everything from 1 up to it is still owed. NULL means the
    // folder owes nothing, which is what every folder synced before this
    // column existed was: those were walked upwards from UID 1.
    r#"
ALTER TABLE folder ADD COLUMN backfill_uid INTEGER;
"#,
    // v18 — who a letter was *to*, in the search index.
    //
    // The index held the subject, the sender and the body, which is most of a
    // letter but not the part that matters in Sent: there, the only name worth
    // searching for is the one it went to. Searching for `cadus` found
    // twenty-six fewer messages than Thunderbird did on the same mailbox,
    // every one of them sent by the person doing the search.
    //
    // The body text lives only here — it is not a column on `message` — so
    // this carries the old rows across rather than rebuilding from scratch,
    // which would silently empty every body. The recipients come from
    // `message`, where they have been all along.
    r#"
CREATE VIRTUAL TABLE message_fts_next USING fts5(subject, sender, recipients, body);
INSERT INTO message_fts_next (rowid, subject, sender, recipients, body)
SELECT f.rowid, f.subject, f.sender, COALESCE(m.recipients, ''), f.body
  FROM message_fts f
  LEFT JOIN message m ON m.id = f.rowid;
DROP TRIGGER message_fts_delete;
DROP TABLE message_fts;
ALTER TABLE message_fts_next RENAME TO message_fts;
CREATE TRIGGER message_fts_delete AFTER DELETE ON message BEGIN
    DELETE FROM message_fts WHERE rowid = old.id;
END;
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
