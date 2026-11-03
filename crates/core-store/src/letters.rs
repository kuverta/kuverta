//! Letters posted on paper.
//!
//! The ledger of what went to LetterXpress: which document, how it was to be
//! printed, what it was quoted at, and what LetterXpress made of it — the
//! job, the address it read off the page, what it cost, where it is.
//!
//! A letter moves `sending` → `sent`, or to `failed` with the reason, and a
//! sent one to `cancelled` if it is taken back in time. `sending` is written
//! before LetterXpress is asked, as the outbox does: a row left in it after
//! a crash says a letter may have gone, which is the truth, and nothing
//! sends it again by itself. The store does not know what a spec is —
//! core-rpc writes it as JSON and reads it back.

use rusqlite::{params, OptionalExtension};

use crate::model::AccountId;
use crate::{Result, Store};

#[derive(Debug, Clone, PartialEq)]
pub struct StoredLetter {
    pub id: i64,
    pub account_id: AccountId,
    pub document_id: Option<i64>,
    pub name: String,
    /// `test` or `live`.
    pub mode: String,
    /// How it was to be printed and posted, as JSON core-rpc wrote.
    pub spec: String,
    pub pages: i64,
    /// What LetterXpress quoted before it was sent.
    pub quoted: Option<f64>,
    /// `sending`, `sent`, `failed` or `cancelled`.
    pub state: String,
    pub job_id: Option<i64>,
    /// LetterXpress's own word for where it is.
    pub job_status: Option<String>,
    /// The address as LetterXpress read it off the page.
    pub address: Option<String>,
    /// What it cost, VAT included.
    pub cost: Option<f64>,
    pub tracking_code: Option<String>,
    pub tracking_status: Option<String>,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// What a letter is before it goes.
pub struct NewLetter<'a> {
    pub account_id: AccountId,
    pub document_id: Option<i64>,
    pub name: &'a str,
    pub mode: &'a str,
    pub spec: &'a str,
    pub pages: i64,
    pub quoted: Option<f64>,
}

/// What LetterXpress said about a job.
#[derive(Debug, Clone, Default)]
pub struct LetterJob<'a> {
    pub job_id: i64,
    pub job_status: &'a str,
    pub address: Option<&'a str>,
    pub cost: Option<f64>,
    pub tracking_code: Option<&'a str>,
    pub tracking_status: Option<&'a str>,
}

const COLUMNS: &str = "id, account_id, document_id, name, mode, spec, pages, quoted, state, \
                       job_id, job_status, address, cost, tracking_code, tracking_status, \
                       last_error, created_at, updated_at";

fn row_to_letter(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredLetter> {
    Ok(StoredLetter {
        id: row.get(0)?,
        account_id: row.get(1)?,
        document_id: row.get(2)?,
        name: row.get(3)?,
        mode: row.get(4)?,
        spec: row.get(5)?,
        pages: row.get(6)?,
        quoted: row.get(7)?,
        state: row.get(8)?,
        job_id: row.get(9)?,
        job_status: row.get(10)?,
        address: row.get(11)?,
        cost: row.get(12)?,
        tracking_code: row.get(13)?,
        tracking_status: row.get(14)?,
        last_error: row.get(15)?,
        created_at: row.get(16)?,
        updated_at: row.get(17)?,
    })
}

impl Store {
    /// Writes a letter down as `sending`, before LetterXpress is asked.
    pub fn begin_letter(&self, letter: &NewLetter<'_>) -> Result<i64> {
        let now = crate::now();
        self.conn.execute(
            "INSERT INTO letter (account_id, document_id, name, mode, spec, pages, quoted,
                                 state, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'sending', ?8, ?8)",
            params![
                letter.account_id,
                letter.document_id,
                letter.name,
                letter.mode,
                letter.spec,
                letter.pages,
                letter.quoted,
                now
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Records what LetterXpress said about the job: when it took the
    /// letter, and every time it is asked after it since. A letter taken
    /// back stays `cancelled` whatever is said later.
    pub fn record_letter_job(&self, id: i64, job: &LetterJob<'_>) -> Result<()> {
        self.conn.execute(
            "UPDATE letter SET
                 state = CASE WHEN state = 'cancelled' THEN state
                              WHEN ?3 = 'canceled' THEN 'cancelled'
                              ELSE 'sent' END,
                 job_id = ?2, job_status = ?3,
                 address = COALESCE(?4, address), cost = COALESCE(?5, cost),
                 tracking_code = COALESCE(?6, tracking_code),
                 tracking_status = COALESCE(?7, tracking_status),
                 last_error = NULL, updated_at = ?8
             WHERE id = ?1",
            params![
                id,
                job.job_id,
                job.job_status,
                job.address,
                job.cost,
                job.tracking_code,
                job.tracking_status,
                crate::now()
            ],
        )?;
        Ok(())
    }

    /// LetterXpress refused it, or could not be reached.
    pub fn letter_failed(&self, id: i64, error: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE letter SET state = 'failed', last_error = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, error, crate::now()],
        )?;
        Ok(())
    }

    pub fn letter_cancelled(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE letter SET state = 'cancelled', updated_at = ?2 WHERE id = ?1",
            params![id, crate::now()],
        )?;
        Ok(())
    }

    pub fn letter(&self, id: i64) -> Result<Option<StoredLetter>> {
        self.conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM letter WHERE id = ?1"),
                params![id],
                row_to_letter,
            )
            .optional()
            .map_err(Into::into)
    }

    /// An account's letters, newest first.
    pub fn letters(&self, account_id: AccountId) -> Result<Vec<StoredLetter>> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM letter WHERE account_id = ?1 ORDER BY created_at DESC, id DESC"
        ))?;
        let rows = statement.query_map(params![account_id], row_to_letter)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// The letters sent from one document, newest first: what its card says
    /// about having been posted.
    pub fn letters_of_document(&self, document_id: i64) -> Result<Vec<StoredLetter>> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM letter WHERE document_id = ?1 ORDER BY created_at DESC, id DESC"
        ))?;
        let rows = statement.query_map(params![document_id], row_to_letter)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documents::NewDocument;
    use crate::model::{ImapSecurity, NewAccount};

    fn account(store: &Store) -> AccountId {
        store
            .add_account(&NewAccount {
                label: "Dev".into(),
                email: "dev@kuverta.test".into(),
                imap_host: "127.0.0.1".into(),
                imap_port: 993,
                imap_security: ImapSecurity::Tls,
                username: "dev@kuverta.test".into(),
                auth_method: "app_password".into(),
                ..Default::default()
            })
            .unwrap()
    }

    #[test]
    fn a_letter_is_written_down_before_it_goes_and_outlives_its_document() {
        let store = Store::open_in_memory().unwrap();
        let account = account(&store);
        let document = store
            .add_document(&NewDocument {
                account_id: account,
                name: "Kündigung.pdf",
                note: None,
                signed: true,
                pages: Some(1),
                pdf: b"%PDF-1.5",
                signed_from: None,
            })
            .unwrap();
        let id = store
            .begin_letter(&NewLetter {
                account_id: account,
                document_id: Some(document),
                name: "Kündigung.pdf",
                mode: "test",
                spec: "{}",
                pages: 1,
                quoted: Some(0.81),
            })
            .unwrap();
        assert_eq!(store.letter(id).unwrap().unwrap().state, "sending");

        store
            .record_letter_job(
                id,
                &LetterJob {
                    job_id: 6035143,
                    job_status: "draft",
                    address: Some("Erika Mustermann, Heidestr. 17, 51147 Köln"),
                    cost: Some(0.81),
                    ..Default::default()
                },
            )
            .unwrap();
        // Asking again later knows no more about the address; what was read
        // the first time stays.
        store
            .record_letter_job(
                id,
                &LetterJob {
                    job_id: 6035143,
                    job_status: "done",
                    tracking_code: Some("RC123456789DE"),
                    ..Default::default()
                },
            )
            .unwrap();
        let letter = store.letter(id).unwrap().unwrap();
        assert_eq!(letter.state, "sent");
        assert_eq!(letter.job_status.as_deref(), Some("done"));
        assert_eq!(
            letter.address.as_deref(),
            Some("Erika Mustermann, Heidestr. 17, 51147 Köln")
        );
        assert_eq!(letter.tracking_code.as_deref(), Some("RC123456789DE"));
        assert_eq!(store.letters_of_document(document).unwrap().len(), 1);

        store.delete_document(document).unwrap();
        let kept = store.letters(account).unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].document_id, None);
    }

    #[test]
    fn a_letter_taken_back_stays_taken_back() {
        let store = Store::open_in_memory().unwrap();
        let account = account(&store);
        let new = NewLetter {
            account_id: account,
            document_id: None,
            name: "Brief.pdf",
            mode: "live",
            spec: "{}",
            pages: 2,
            quoted: None,
        };
        let id = store.begin_letter(&new).unwrap();
        store.letter_cancelled(id).unwrap();
        store
            .record_letter_job(
                id,
                &LetterJob {
                    job_id: 1,
                    job_status: "queue",
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(store.letter(id).unwrap().unwrap().state, "cancelled");

        let failed = store.begin_letter(&new).unwrap();
        store.letter_failed(failed, "Unauthorized.").unwrap();
        let letter = store.letter(failed).unwrap().unwrap();
        assert_eq!(letter.state, "failed");
        assert_eq!(letter.last_error.as_deref(), Some("Unauthorized."));
    }
}
