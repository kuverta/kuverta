//! What has already been done with a thing found in mail.
//!
//! An invitation arrives more than once — forwarded, synced again, or filed
//! under two of Gmail's labels — and the mail it arrives in is a different
//! message every time. So the ledger is keyed by what identifies the thing
//! itself, which for an invitation is its `UID`. Reading a folder twice is
//! then as harmless as reading it once, which is the property the whole
//! offer half rests on.
//!
//! What it is not is a record of the mail: a message row already knows
//! which mail it is. This knows that *this event* was accepted, so that the
//! second copy of the invitation can say so instead of asking again.

use rusqlite::{params, OptionalExtension};

use crate::{Result, Store};

/// One thing acted on, and where it went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handoff {
    /// What identifies the thing itself, not the mail that carried it.
    pub key: String,
    /// Which revision of it was acted on. An organiser who moves a meeting
    /// sends the same `UID` with a higher `SEQUENCE`; that is a new
    /// question, and a row written at the old revision does not answer it.
    pub revision: i64,
    /// What was done: accepted, declined, or whatever the kind allows.
    pub answer: String,
    /// Where it was handed to, once anywhere takes it. `None` means
    /// nothing outside kuverta was touched.
    pub sink: Option<String>,
    /// What that place calls what it made, so the same thing can be found
    /// again — to take an event back out when the meeting is cancelled.
    pub reference: Option<String>,
    pub at: i64,
}

impl Store {
    /// What was done with this thing, if anything.
    pub fn handoff(&self, key: &str) -> Result<Option<Handoff>> {
        self.conn
            .query_row(
                "SELECT key, revision, answer, sink, reference, at
                 FROM handoff WHERE key = ?1",
                params![key],
                |row| {
                    Ok(Handoff {
                        key: row.get(0)?,
                        revision: row.get(1)?,
                        answer: row.get(2)?,
                        sink: row.get(3)?,
                        reference: row.get(4)?,
                        at: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// Whether this revision of the thing has been dealt with already.
    ///
    /// The question a caller actually has, and the reason `revision` is in
    /// the table: an invitation re-sent unchanged is answered, and the same
    /// invitation re-sent for a moved meeting is not.
    pub fn handled(&self, key: &str, revision: i64) -> Result<bool> {
        Ok(self
            .handoff(key)?
            .is_some_and(|handoff| handoff.revision >= revision))
    }

    /// Record what was done. A later revision replaces an earlier one; an
    /// earlier one is ignored, because mail arrives out of order and the
    /// older copy of an invitation must not undo the answer to the newer.
    pub fn record_handoff(&self, handoff: &Handoff) -> Result<()> {
        self.conn.execute(
            "INSERT INTO handoff (key, revision, answer, sink, reference, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (key) DO UPDATE SET
                 revision = excluded.revision,
                 answer = excluded.answer,
                 sink = excluded.sink,
                 reference = excluded.reference,
                 at = excluded.at
             WHERE excluded.revision >= handoff.revision",
            params![
                handoff.key,
                handoff.revision,
                handoff.answer,
                handoff.sink,
                handoff.reference,
                crate::now(),
            ],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handoff(key: &str, revision: i64, answer: &str) -> Handoff {
        Handoff {
            key: key.to_string(),
            revision,
            answer: answer.to_string(),
            sink: None,
            reference: None,
            at: 0,
        }
    }

    #[test]
    fn nothing_is_handled_until_it_is_recorded() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.handoff("invite:abc").unwrap(), None);
        assert!(!store.handled("invite:abc", 0).unwrap());
    }

    #[test]
    fn the_same_invitation_arriving_again_is_already_handled() {
        let store = Store::open_in_memory().unwrap();
        store
            .record_handoff(&handoff("invite:abc", 2, "accepted"))
            .unwrap();
        assert!(store.handled("invite:abc", 2).unwrap());
        assert!(
            store.handled("invite:abc", 1).unwrap(),
            "an older copy of the same invitation asks nothing new"
        );
    }

    #[test]
    fn a_moved_meeting_is_a_new_question() {
        let store = Store::open_in_memory().unwrap();
        store
            .record_handoff(&handoff("invite:abc", 2, "accepted"))
            .unwrap();
        assert!(
            !store.handled("invite:abc", 3).unwrap(),
            "a higher SEQUENCE is the organiser asking again"
        );
    }

    #[test]
    fn an_older_copy_does_not_undo_the_answer_to_a_newer_one() {
        let store = Store::open_in_memory().unwrap();
        store
            .record_handoff(&handoff("invite:abc", 3, "declined"))
            .unwrap();
        store
            .record_handoff(&handoff("invite:abc", 1, "accepted"))
            .unwrap();
        let kept = store.handoff("invite:abc").unwrap().unwrap();
        assert_eq!(kept.revision, 3);
        assert_eq!(kept.answer, "declined");
    }

    #[test]
    fn where_it_went_is_kept_so_it_can_be_found_again() {
        let store = Store::open_in_memory().unwrap();
        store
            .record_handoff(&Handoff {
                sink: Some("macos-calendar".into()),
                reference: Some("EK-7F3A".into()),
                ..handoff("invite:abc", 1, "accepted")
            })
            .unwrap();
        let kept = store.handoff("invite:abc").unwrap().unwrap();
        assert_eq!(kept.sink.as_deref(), Some("macos-calendar"));
        assert_eq!(kept.reference.as_deref(), Some("EK-7F3A"));
        assert!(
            kept.at > 0,
            "recorded when it happened, not when it was asked"
        );
    }
}
