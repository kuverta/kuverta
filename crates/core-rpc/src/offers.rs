//! What a message is offering, found in its parts.
//!
//! An invitation is a `text/calendar` part, and where it sits in the message
//! is not something to rely on: a calendar program may put it inside a
//! `multipart/alternative` beside the text and the HTML, attach it as
//! `invite.ics`, or both. So every part is looked at by its own declared
//! type rather than asking which of them the parser took for a body — the
//! same invitation must be found whichever shape it arrives in.
//!
//! Whether it may be answered turns on one question, and the answer comes
//! from the mail there is: **has this mailbox ever written to the sender?**
//!
//! Not "have they written to us", which is the obvious test and the wrong
//! one. Inbound volume is the sender's to manufacture — a newsletter has
//! written fifty times and is still a stranger, and an unsolicited invitation
//! is fishing for exactly the confirmation that answering it would give. A
//! letter *sent* from this mailbox is the one signal nobody outside it can
//! produce. So an invitation from someone written to before carries its
//! buttons, and every other invitation is shown without them and says why.

use mail_parser::{Message as MimeMessage, MessagePart, MimeHeaders};
use serde::{Deserialize, Serialize};

use core_offers::{invites, Detail, Offer, Origin, Standing};

use crate::{AccountId, Core, Result};

/// One offer, with what the ledger already knows about it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OfferView {
    pub offer: Offer,
    /// This revision of it has been dealt with: the second copy of an
    /// invitation already accepted says so instead of asking again. An
    /// organiser who moves the meeting raises its `SEQUENCE`, and that is a
    /// new question rather than a repeat.
    pub handled: bool,
}

/// Every `text/calendar` part, by its declared type.
fn calendar_parts<'a>(parsed: &'a MimeMessage<'a>) -> impl Iterator<Item = &'a MessagePart<'a>> {
    parsed.parts.iter().filter(|part| {
        if part.is_multipart() {
            return false;
        }
        part.content_type().is_some_and(|ct| {
            let subtype = ct.subtype().unwrap_or_default().to_ascii_lowercase();
            (ct.ctype().eq_ignore_ascii_case("text") && subtype == "calendar")
                || (ct.ctype().eq_ignore_ascii_case("application")
                    && (subtype == "ics" || subtype == "calendar"))
        })
    })
}

impl Core {
    /// What the message is offering. Empty for almost all mail, which is
    /// why it costs a walk over the parts already in hand and nothing else.
    pub(crate) fn offers_in(
        &self,
        account: AccountId,
        parsed: &MimeMessage<'_>,
        origin: Origin,
        from: Option<&str>,
    ) -> Result<Vec<OfferView>> {
        let mut found = Vec::new();
        for part in calendar_parts(parsed) {
            let Some(text) = part.text_contents() else {
                continue;
            };
            let Some(invite) = invites::read(text) else {
                continue;
            };
            let standing = self.standing_of(account, from)?;
            let offer = Offer::from_invite(origin, invite, standing);
            // The same invitation can be in a message twice — once in the
            // alternative and once attached — and it is one offer.
            if found
                .iter()
                .any(|seen: &OfferView| seen.offer.key == offer.key)
            {
                continue;
            }
            let revision = match &offer.detail {
                Detail::Invite(invite) => invite.sequence.unwrap_or(0),
            };
            let handled = self.store.handled(&offer.key, revision)?;
            found.push(OfferView { offer, handled });
        }
        Ok(found)
    }

    /// Whether the mailbox has written to this address before.
    fn standing_of(&self, account: AccountId, from: Option<&str>) -> Result<Standing> {
        let Some(address) = from.map(str::trim).filter(|from| from.contains('@')) else {
            return Ok(Standing::Stranger);
        };
        let me = self.account_email(account)?;
        let written = self.store.times_written_to(account, &me, address)?;
        Ok(if written > 0 {
            Standing::Known
        } else {
            Standing::Stranger
        })
    }
}
