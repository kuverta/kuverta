//! Things found in mail, and offered to the person.
//!
//! An invitation is the first of them: a `text/calendar` part the sender's
//! program wrote, which kuverta can read and show. A todo taken from a
//! sentence, and a line off a bank's statement, are the same sort of thing
//! and are not here yet.
//!
//! The shape is the point. What one kind needs and another does not sits in
//! [`Detail`]; what they all have — where it was found, what identifies it,
//! whether its sender is a stranger, whether it may be answered — sits in
//! [`Offer`]. That is the trick `core-paper` plays by giving a document the
//! shapes a message has: unify on the data rather than on an interface, so
//! the surface above learns one thing once.
//!
//! Two rules hold all of it, and both are easier to keep now than to add
//! later:
//!
//! - **An offer is offered, never applied.** Reading mail creates nothing
//!   outside kuverta. A person accepts, and only then does anything leave.
//!   Syncing a folder twice must therefore be as harmless as syncing it
//!   once, which is what the handoff ledger in `core-store` is for.
//! - **A stranger's offer carries no buttons.** Answering an invitation
//!   tells whoever sent it that the address is read by somebody, which is
//!   exactly what an unsolicited one is fishing for. So the answer is
//!   withheld where there is no correspondence to justify it, and the card
//!   says why rather than quietly doing nothing.
//!
//! What this crate deliberately has not got is a sink. Nothing here writes
//! to a calendar, a reminder list or an archive. Where an accepted offer
//! goes is a separate question with a separate answer per platform, and
//! keeping it out means this half stays the same on all of them.

pub mod invites;

use serde::{Deserialize, Serialize};

/// Where an offer was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", content = "id", rename_all = "snake_case")]
pub enum Origin {
    /// A message in the store, by its row id.
    Message(i64),
    /// A document in Paperless, by the id it has there.
    ///
    /// Nothing produces one yet. It is here because the first kind that
    /// reads paper will, and because an `Origin` that could only ever be a
    /// message id would be a message id with extra words.
    Document(i64),
}

/// Whether the person has had anything to do with whoever sent this.
///
/// Decided from the mail there is, not from a list anyone maintains: the
/// same source the To field suggests from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Standing {
    /// An address this mailbox has written to, or heard from before.
    Known,
    /// An address that appears here for the first time.
    Stranger,
}

/// Why an offer may be read but not answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Withheld {
    /// The sender is a stranger, and an answer would tell them the address
    /// is live. The invitation is still shown; only the answer is held back.
    Stranger,
    /// There is nothing to answer: a published event, somebody else's reply,
    /// or a cancellation, which is news rather than a question.
    NothingAsked,
}

/// What was found, in the shape its own kind needs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Detail {
    Invite(invites::Invite),
}

/// One thing found in mail, as the reading pane shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Offer {
    pub origin: Origin,
    /// What identifies the thing itself rather than the mail that carried
    /// it, so that the same invitation arriving twice — forwarded, synced
    /// again, or filed under two of Gmail's labels — is recognised as one.
    /// Falls back to the origin when the sender gave nothing to go on.
    pub key: String,
    /// One line, for the head of the card.
    pub title: String,
    pub standing: Standing,
    /// `None` when the offer could be answered, which is not the same as
    /// saying kuverta can answer it yet.
    pub withheld: Option<Withheld>,
    pub detail: Detail,
}

impl Offer {
    /// An invitation, as the pane shows it.
    pub fn from_invite(origin: Origin, invite: invites::Invite, standing: Standing) -> Self {
        let key = match invite.uid.as_deref() {
            Some(uid) if !uid.trim().is_empty() => format!("invite:{}", uid.trim()),
            _ => match origin {
                Origin::Message(id) => format!("invite:message:{id}"),
                Origin::Document(id) => format!("invite:document:{id}"),
            },
        };
        let withheld = if !invite.intent.asks_an_answer() {
            Some(Withheld::NothingAsked)
        } else if standing == Standing::Stranger {
            Some(Withheld::Stranger)
        } else {
            None
        };
        let title = invite.summary.clone().unwrap_or_default();
        Self {
            origin,
            key,
            title,
            standing,
            withheld,
            detail: Detail::Invite(invite),
        }
    }

    /// Whether an answer may be sent, once there is something to send it
    /// with.
    pub fn answerable(&self) -> bool {
        self.withheld.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invites::{Intent, Invite};

    fn invite(uid: Option<&str>, intent: Intent) -> Invite {
        Invite {
            intent,
            uid: uid.map(str::to_string),
            summary: Some("Quartalsbesprechung".into()),
            ..Invite::default()
        }
    }

    #[test]
    fn the_same_invitation_twice_is_one_key() {
        let first = Offer::from_invite(
            Origin::Message(1),
            invite(Some("abc-123"), Intent::Request),
            Standing::Known,
        );
        let again = Offer::from_invite(
            Origin::Message(2),
            invite(Some(" abc-123 "), Intent::Request),
            Standing::Known,
        );
        assert_eq!(
            first.key, again.key,
            "a UID identifies the event, not the mail"
        );
    }

    #[test]
    fn an_invitation_without_a_uid_falls_back_to_where_it_was_found() {
        let offer = Offer::from_invite(
            Origin::Message(7),
            invite(None, Intent::Request),
            Standing::Known,
        );
        assert_eq!(offer.key, "invite:message:7");
    }

    #[test]
    fn a_strangers_invitation_is_shown_but_not_answerable() {
        let offer = Offer::from_invite(
            Origin::Message(1),
            invite(Some("x"), Intent::Request),
            Standing::Stranger,
        );
        assert!(!offer.answerable());
        assert_eq!(offer.withheld, Some(Withheld::Stranger));
        assert_eq!(offer.title, "Quartalsbesprechung", "it is still read");
    }

    #[test]
    fn a_cancellation_asks_nothing_even_from_someone_known() {
        let offer = Offer::from_invite(
            Origin::Message(1),
            invite(Some("x"), Intent::Cancel),
            Standing::Known,
        );
        assert_eq!(offer.withheld, Some(Withheld::NothingAsked));
    }

    #[test]
    fn an_invitation_from_a_correspondent_is_answerable() {
        let offer = Offer::from_invite(
            Origin::Message(1),
            invite(Some("x"), Intent::Request),
            Standing::Known,
        );
        assert!(offer.answerable());
    }
}
