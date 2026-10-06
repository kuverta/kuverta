//! Documents: PDFs kuverta made, and the signature it puts on them.
//!
//! The assistant writes documents — a letter, a cancellation, a confirmation
//! — and signs PDFs: its own, or one that came as an attachment. Each is kept
//! here, bytes and all, on the account it was made for, so the person can
//! open, save, attach or throw it away later, and the assistant can list and
//! read them again.
//!
//! The signature is one picture for the whole store: a PNG or JPEG of the
//! person's handwritten signature, with the place that goes in front of the
//! date beneath it. Only its presence and its measurements are reported from
//! the listing; the picture itself is fetched on purpose.

use rusqlite::{params, OptionalExtension};

use crate::model::AccountId;
use crate::{Result, Store};

/// A document as the store lists it: everything but the bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredDocument {
    pub id: i64,
    pub account_id: AccountId,
    /// A file name, with its extension.
    pub name: String,
    /// What it is, in the assistant's words: "signed copy of Mietvertrag.pdf
    /// from Erika Mustermann".
    pub note: Option<String>,
    pub signed: bool,
    pub pages: Option<i64>,
    pub size: i64,
    pub created_at: i64,
}

/// A document to keep.
#[derive(Debug, Clone)]
pub struct NewDocument<'a> {
    pub account_id: AccountId,
    pub name: &'a str,
    pub note: Option<&'a str>,
    pub signed: bool,
    pub pages: Option<i64>,
    pub pdf: &'a [u8],
}

/// The signature as stored. `image` is `None` when only the place is set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSignature {
    pub image: Option<Vec<u8>>,
    pub content_type: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub place: Option<String>,
    pub updated_at: i64,
}

impl Store {
    pub fn add_document(&self, new: &NewDocument<'_>) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO document (account_id, name, note, signed, pages, pdf, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                new.account_id,
                new.name,
                new.note,
                new.signed,
                new.pages,
                new.pdf,
                crate::now()
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// The account's documents, newest first.
    pub fn documents(&self, account_id: AccountId) -> Result<Vec<StoredDocument>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, account_id, name, note, signed, pages, length(pdf), created_at
               FROM document WHERE account_id = ?1
              ORDER BY created_at DESC, id DESC",
        )?;
        let rows = stmt.query_map(params![account_id], document_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn document(&self, id: i64) -> Result<Option<StoredDocument>> {
        self.conn
            .query_row(
                "SELECT id, account_id, name, note, signed, pages, length(pdf), created_at
                   FROM document WHERE id = ?1",
                params![id],
                document_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn document_bytes(&self, id: i64) -> Result<Option<Vec<u8>>> {
        self.conn
            .query_row(
                "SELECT pdf FROM document WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn rename_document(&self, id: i64, name: &str) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE document SET name = ?2 WHERE id = ?1",
            params![id, name],
        )?;
        Ok(changed > 0)
    }

    pub fn delete_document(&self, id: i64) -> Result<bool> {
        let deleted = self
            .conn
            .execute("DELETE FROM document WHERE id = ?1", params![id])?;
        Ok(deleted > 0)
    }

    pub fn signature(&self) -> Result<Option<StoredSignature>> {
        self.conn
            .query_row(
                "SELECT image, content_type, width, height, place, updated_at FROM signature WHERE id = 1",
                [],
                |row| {
                    Ok(StoredSignature {
                        image: row.get(0)?,
                        content_type: row.get(1)?,
                        width: row.get(2)?,
                        height: row.get(3)?,
                        place: row.get(4)?,
                        updated_at: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// Keeps the picture; the place stays as it was.
    pub fn set_signature(
        &self,
        image: &[u8],
        content_type: &str,
        width: i64,
        height: i64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO signature (id, image, content_type, width, height, updated_at)
             VALUES (1, ?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (id) DO UPDATE SET image = excluded.image,
                 content_type = excluded.content_type, width = excluded.width,
                 height = excluded.height, updated_at = excluded.updated_at",
            params![image, content_type, width, height, crate::now()],
        )?;
        Ok(())
    }

    /// The place written in front of the date beneath the signature. Empty
    /// clears it.
    pub fn set_signature_place(&self, place: Option<&str>) -> Result<()> {
        let place = place.map(str::trim).filter(|p| !p.is_empty());
        self.conn.execute(
            "INSERT INTO signature (id, place, updated_at) VALUES (1, ?1, ?2)
             ON CONFLICT (id) DO UPDATE SET place = excluded.place, updated_at = excluded.updated_at",
            params![place, crate::now()],
        )?;
        Ok(())
    }

    /// Forgets the picture. The place stays: it is not a secret, and the next
    /// picture wants it too.
    pub fn clear_signature(&self) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE signature SET image = NULL, content_type = NULL, width = NULL, height = NULL,
                    updated_at = ?1 WHERE id = 1 AND image IS NOT NULL",
            params![crate::now()],
        )?;
        Ok(changed > 0)
    }
}

fn document_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredDocument> {
    Ok(StoredDocument {
        id: row.get(0)?,
        account_id: row.get(1)?,
        name: row.get(2)?,
        note: row.get(3)?,
        signed: row.get(4)?,
        pages: row.get(5)?,
        size: row.get(6)?,
        created_at: row.get(7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn documents_are_kept_listed_renamed_and_deleted_with_their_bytes() {
        let store = Store::open_in_memory().unwrap();
        let account = account(&store);
        let id = store
            .add_document(&NewDocument {
                account_id: account,
                name: "Kuendigung.pdf",
                note: Some("a cancellation"),
                signed: true,
                pages: Some(1),
                pdf: b"%PDF-1.5 x",
            })
            .unwrap();
        let listed = store.documents(account).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, id);
        assert_eq!(listed[0].size, 10);
        assert!(listed[0].signed);
        assert_eq!(store.document_bytes(id).unwrap().unwrap(), b"%PDF-1.5 x");
        assert!(store
            .rename_document(id, "Kuendigung Fitnessstudio.pdf")
            .unwrap());
        assert_eq!(
            store.document(id).unwrap().unwrap().name,
            "Kuendigung Fitnessstudio.pdf"
        );
        assert!(store.delete_document(id).unwrap());
        assert!(!store.delete_document(id).unwrap());
        assert!(store.documents(account).unwrap().is_empty());
    }

    #[test]
    fn documents_go_with_their_account() {
        let store = Store::open_in_memory().unwrap();
        let account = account(&store);
        store
            .add_document(&NewDocument {
                account_id: account,
                name: "x.pdf",
                note: None,
                signed: false,
                pages: None,
                pdf: b"%PDF",
            })
            .unwrap();
        store.delete_account(account).unwrap();
        assert!(store.documents(account).unwrap().is_empty());
    }

    #[test]
    fn the_signature_is_one_picture_and_a_place() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.signature().unwrap().is_none());
        store.set_signature_place(Some(" Musterstadt ")).unwrap();
        let only_place = store.signature().unwrap().unwrap();
        assert!(only_place.image.is_none());
        assert_eq!(only_place.place.as_deref(), Some("Musterstadt"));
        store.set_signature(b"png", "image/png", 300, 100).unwrap();
        let both = store.signature().unwrap().unwrap();
        assert_eq!(both.image.as_deref(), Some(&b"png"[..]));
        assert_eq!(both.width, Some(300));
        assert_eq!(both.place.as_deref(), Some("Musterstadt"));
        assert!(store.clear_signature().unwrap());
        assert!(!store.clear_signature().unwrap());
        let cleared = store.signature().unwrap().unwrap();
        assert!(cleared.image.is_none());
        assert_eq!(cleared.place.as_deref(), Some("Musterstadt"));
        store.set_signature_place(Some("")).unwrap();
        assert!(store.signature().unwrap().unwrap().place.is_none());
    }
}
