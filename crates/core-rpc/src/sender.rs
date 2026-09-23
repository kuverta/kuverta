//! Who the message is from, summarised.
//!
//! A message tells you what somebody wrote; it does not tell you who they
//! are to you. The card this builds answers the question the sender line
//! raises and the sender line cannot hold: how long you have had mail with
//! them, who spoke last and when, what their mail usually is, where it ends
//! up filed, and whether any of it is still waiting on you — followed by the
//! last few messages either way, which is the "last correspondence".
//!
//! It is asked for by message, not by address: the window has a row under the
//! cursor, not an address, and who the other person is in a message the
//! account itself sent is its first recipient rather than its sender. The
//! store settles that with the same rule the conversation list groups by, so
//! the card and the conversation are always about the same person. An address
//! may be passed instead, which is what the People list has to hand.
//!
//! Post is asked about the same way and answered in the same shape — see
//! [`Core::paper_sender`] — so the window has one card to draw rather than
//! two that would drift apart.

use core_store::model::{AccountId, MessageId};
use core_store::Correspondent;
use serde::{Deserialize, Serialize};

use crate::urgency::UrgencyView;
use crate::{Core, Result, RpcError};

/// How many messages of the exchange the card carries. Enough for the window
/// to show three in a hover card and all of them in the sidebar, and few
/// enough that hovering down a list does not read a mailbox per row.
pub(crate) const RECENT: usize = 6;

/// One message of the recent exchange, in either direction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SenderMessage {
    pub id: MessageId,
    /// Whether the account wrote it.
    pub from_me: bool,
    pub date_utc: Option<i64>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub unread: bool,
}

/// A folder their mail is in, and how much of it is there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SenderFolder {
    pub name: String,
    pub messages: usize,
}

/// What is known about the person a message is with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SenderView {
    /// Their e-mail address. Empty for a scanned letter, which has none.
    pub address: String,
    /// Their postal address, for a scanned letter whose page gives one up.
    /// Empty for mail, which does not carry one. See
    /// [`core_paper::Document::postal_address`].
    pub postal: Option<String>,
    /// Their name as they last wrote it, falling back to the address so the
    /// window never has to decide what to show when there is no name.
    pub name: String,
    pub received: usize,
    pub sent: usize,
    pub unread: usize,
    pub first_utc: Option<i64>,
    pub last_from_them_utc: Option<i64>,
    pub last_to_them_utc: Option<i64>,
    pub with_attachments: usize,
    /// What their mail is most often classified as, and how much of it is.
    pub category: Option<String>,
    pub category_count: usize,
    /// Their mail is bulk — a newsletter, marketing, a notification — which
    /// is worth saying plainly: it is the difference between somebody who
    /// has written to you fifty times and a shop that has mailed you fifty
    /// times.
    pub bulk: bool,
    pub folders: Vec<SenderFolder>,
    /// The most pressing verdict on their Inbox mail, when there is one.
    pub waiting: Option<UrgencyView>,
    /// The last few messages either way, newest first.
    pub recent: Vec<SenderMessage>,
}

impl Core {
    /// The card for whoever sent a scanned letter.
    ///
    /// The same shape as a message's, so the window draws one card: what
    /// changes is that a letter has a postal address instead of an e-mail
    /// one, nothing was ever sent back, and "filed in" is the folders on the
    /// shelf rather than folders on a server.
    pub async fn paper_sender(&self, mailbox: i64, document: i64) -> Result<SenderView> {
        self.paper_session(mailbox)?.sender_card(document).await
    }

    /// The card for the person behind a message, or behind an address.
    ///
    /// `message` is what the list and the reading pane have; `address` is what
    /// the People list has. One of the two is needed.
    pub fn sender(
        &self,
        account: AccountId,
        message: Option<MessageId>,
        address: Option<&str>,
    ) -> Result<SenderView> {
        let me = self.account_email(account)?;
        let who = match address
            .map(|a| a.trim().to_lowercase())
            .filter(|a| !a.is_empty())
        {
            Some(address) => address,
            None => {
                let id = message.ok_or_else(|| {
                    RpcError::Rejected("a sender is asked for by message or by address".into())
                })?;
                self.store()
                    .counterpart_of(account, &me, id)?
                    .ok_or(RpcError::UnknownMessage(id))?
            }
        };

        let skip = self.folders_to_leave_out(account)?;
        let known = self.store().correspondent(account, &me, &who, &skip)?;
        let recent = self
            .store()
            .conversation(account, &me, &who, &skip, RECENT)?
            .into_iter()
            .rev()
            .map(|m| SenderMessage {
                id: m.id,
                from_me: m.from_me,
                date_utc: m.date_utc,
                subject: m.subject,
                snippet: m.snippet,
                unread: m.unread,
            })
            .collect();

        Ok(view(known, recent))
    }
}

fn view(known: Correspondent, recent: Vec<SenderMessage>) -> SenderView {
    let (category, category_count) = match known.usual_category {
        Some((category, count)) => (Some(category), count),
        None => (None, 0),
    };
    SenderView {
        postal: None,
        bulk: category
            .as_deref()
            .is_some_and(|c| core_store::CLEANUP_CATEGORIES.contains(&c)),
        name: known
            .name
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| known.address.clone()),
        address: known.address,
        received: known.received,
        sent: known.sent,
        unread: known.unread,
        first_utc: known.first_utc,
        last_from_them_utc: known.last_from_them_utc,
        last_to_them_utc: known.last_to_them_utc,
        with_attachments: known.with_attachments,
        category,
        category_count,
        folders: known
            .folders
            .into_iter()
            .map(|(name, messages)| SenderFolder { name, messages })
            .collect(),
        waiting: known.waiting.map(Into::into),
        recent,
    }
}
