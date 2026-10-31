// An invitation a message carries, shown above its text.
//
// A `text/calendar` part used to be a 📅 in the attachment row and nothing
// else: to find out what the meeting was, you opened the .ics in another
// program. The card says it here — what, when, where, who else — because
// that is the whole of what most invitation mail is for.
//
// Three things it will not do, each for a reason worth keeping:
//
// **It never shows a time the sender did not give.** An invitation written
// `TZID=Europe/Berlin:20261104T140000` is a wall clock, and which instant
// that is depends on a zone database the core does not carry. So the card
// shows the digits the sender wrote and names their zone beside them, and
// only formats a real date when the core hands back an instant. A card
// showing a confident wrong hour is worse than one showing none.
//
// **It offers no buttons yet.** Answering means sending mail back as the
// person, and placing the event means reaching outside the app — both have
// their own decisions still to make. The card says so, once, rather than
// showing Accept and failing.
//
// **A stranger's invitation gets no answer.** Replying tells whoever sent it
// that the address is read by somebody, which is what an unsolicited one is
// fishing for. The core decides this — see core-rpc's offers.rs, where the
// test is whether this mailbox has ever *written* to them — and the card
// explains it rather than leaving a dead space where the buttons would be.

const inviteCard = el("reading-invite");

/// The invitation in an open message, or nothing. `null` clears the card,
/// which is what opening a letter or a message without one does.
function showInvite(detail) {
  inviteCard.hidden = true;
  inviteCard.textContent = "";
  const shown = (detail?.offers ?? []).find((offer) => offer.offer.detail.kind === "invite");
  if (!shown) return;

  const invite = shown.offer.detail;
  const head = document.createElement("div");
  head.className = "invite-head";
  head.textContent = shown.offer.title || t("(no title)");
  inviteCard.append(head);

  const lines = [];
  const when = inviteWhen(invite);
  if (when) lines.push([t("When"), when]);
  if (invite.location) lines.push([t("Where"), invite.location]);
  const organizer = invitePerson(invite.organizer);
  if (organizer) lines.push([t("From"), organizer]);
  const others = (invite.attendees ?? []).map(invitePerson).filter(Boolean);
  if (others.length) lines.push([t("Who"), others.join(", ")]);

  const table = document.createElement("dl");
  table.className = "invite-rows";
  for (const [label, value] of lines) {
    const key = document.createElement("dt");
    key.textContent = label;
    const said = document.createElement("dd");
    said.textContent = value;
    table.append(key, said);
  }
  inviteCard.append(table);

  for (const note of inviteNotes(shown)) {
    const line = document.createElement("p");
    line.className = "invite-note";
    line.textContent = note;
    inviteCard.append(line);
  }
  inviteCard.hidden = false;
}

/// When it is, in the sender's own terms.
function inviteWhen(invite) {
  const begins = invite.begins;
  if (!begins) return "";
  // An instant the core vouched for: shown the way every other date in the
  // window is shown.
  if (begins.at) {
    const ends = invite.ends?.at;
    const shown = formatDate(begins.at);
    return ends ? t("{from} until {to}", { from: shown, to: formatDate(ends) }) : shown;
  }
  let said = begins.all_day ? t("{date}, all day", { date: begins.written }) : begins.written;
  if (begins.zone) said = t("{time} ({zone})", { time: said, zone: begins.zone });
  if (invite.repeats) said = t("{time} — repeats", { time: said });
  return said;
}

function invitePerson(person) {
  if (!person) return "";
  if (person.name && person.address) return `${person.name} <${person.address}>`;
  return person.name || person.address || "";
}

/// What the card has to say beyond the event itself: what this invitation
/// is, whether it has been dealt with, and why there are no buttons.
function inviteNotes(shown) {
  const notes = [];
  const kind = shown.offer.detail.intent;
  if (kind === "cancel") notes.push(t("This meeting was cancelled."));
  else if (kind === "reply") notes.push(t("This is somebody's answer to an invitation."));
  else if (kind === "published") notes.push(t("Sent for information — there is nothing to answer."));
  else if (kind === "counter") notes.push(t("Another time is proposed."));

  if (shown.handled) notes.push(t("You have already answered this one."));

  if (shown.offer.withheld === "stranger") {
    notes.push(
      t("No answer is offered: you have never written to this sender, and replying would confirm that someone reads this address."),
    );
  } else if (shown.offer.withheld !== "nothing_asked") {
    notes.push(t("Answering from kuverta is not built yet — open the invitation in your calendar to accept it."));
  }
  return notes;
}
