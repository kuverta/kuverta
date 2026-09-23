// Who the message is with.
//
// A sender line says a name. It does not say whether this is the fortieth
// letter from a shop you never answer or the first from somebody you have
// been writing to for three years — and that is usually the thing that
// decides what to do with the message. This asks the core for the little it
// can know from the mail already in the store and draws it as one card:
// how much mail there is each way, since when, who spoke last, what their
// mail usually is, where it gets filed, whether any of it is still waiting,
// and the last few messages either way.
//
// The same card is drawn in two places. In the right-hand column, above the
// assistant, it belongs to the message that is open; floating beside a list
// row it belongs to whatever the cursor is resting on, which is the cheaper
// question — it answers "who is this?" without opening anything or marking
// anything read.
//
// Post is the same card with a postal address where the e-mail address goes:
// a letter has no address in a header, so the one it shows was read off the
// page, and only where it could not be the reader's own. See
// `core_paper::Document::postal_address`.

const senderPanel = el("sender");
const senderHover = el("sender-hover");

/// How long the cursor must rest on a row before its card appears. Long
/// enough that running down the list with the mouse does not strobe cards,
/// short enough to feel like an answer rather than a wait.
const HOVER_DELAY_MS = 450;

/// Cards already fetched, by account or postbox and by whoever was asked
/// about. The counts move whenever mail does, so this is thrown away on
/// every reload rather than kept fresh — a card is a few small queries.
const senderCache = new Map();
let hoverTimer = null;
/// What the hover card is showing, so moving within one row does not refetch.
let hoverFor = null;

const sinceDate = new Intl.DateTimeFormat(undefined, { month: "long", year: "numeric" });

// -- asking ------------------------------------------------------------------

/// The card for a message, for an address, or for a scanned letter. The list
/// and the reading pane have a message or a document, the People list has an
/// address.
///
/// Post goes to Paperless rather than to the store, and comes back in the
/// same shape: the card itself never learns which it is looking at.
async function senderSummary({ id = null, address = null, postbox = null }) {
  if (postbox === null && state.account === null) return null;
  const key = postbox
    ? `post:${postbox}|${id}`
    : `${state.account}|${address ? `@${address.toLowerCase()}` : id}`;
  if (senderCache.has(key)) return senderCache.get(key);
  const view = postbox
    ? await invoke("paper_sender", { id: postbox, documentId: id })
    : await invoke("sender", { account: state.account, id, address });
  senderCache.set(key, view);
  return view;
}

/// Called wherever the counts could have moved.
function forgetSenders() {
  senderCache.clear();
}

// -- drawing -------------------------------------------------------------------

/// The card itself.
///
/// `interactive` leaves in the parts that are there to be clicked. The hover
/// copy takes no pointer events at all, so a dead button in it would be a
/// button that lies about what it does.
function senderCard(view, { interactive = false, recent = 6, open = openWholeMessage } = {}) {
  const card = document.createDocumentFragment();

  const head = document.createElement("div");
  head.className = "sender-head";
  const avatar = document.createElement("span");
  avatar.className = "avatar";
  avatar.textContent = initials(view.name);
  const who = document.createElement("div");
  who.className = "sender-who";
  const name = document.createElement("div");
  name.className = "sender-name";
  name.textContent = view.name;
  name.title = view.name;
  who.append(name);
  if (view.address) {
    const address = document.createElement("div");
    address.className = "sender-address";
    address.textContent = view.address;
    address.title = view.address;
    who.append(address);
  }
  head.append(avatar, who);
  card.append(head);

  // A letter's address is on the page, not in a header, so it comes as the
  // two or three lines it was printed on and is shown that way.
  if (view.postal) {
    const postal = document.createElement("div");
    postal.className = "sender-postal";
    postal.textContent = view.postal;
    card.append(postal);
  }

  const stats = document.createElement("div");
  stats.className = "sender-stats";
  const counts = [
    [view.received, t("from them")],
    [view.sent, t("from you")],
    [view.unread, t("unread")],
    [view.with_attachments, t("with a file")],
  ];
  for (const [count, label] of counts) {
    if (!count) continue;
    const stat = document.createElement("span");
    const number = document.createElement("b");
    number.textContent = String(count);
    stat.append(number, ` ${label}`);
    stats.append(stat);
  }
  if (!stats.childElementCount) {
    const none = document.createElement("span");
    none.textContent = t("no mail with them yet");
    stats.append(none);
  }
  card.append(stats);

  const lines = document.createElement("div");
  lines.className = "sender-lines";
  const said = (what, value, title) => {
    if (!value) return;
    const line = document.createElement("div");
    line.className = "sender-line";
    const label = document.createElement("span");
    label.className = "what";
    label.textContent = what;
    const text = document.createElement("span");
    text.className = "value";
    text.textContent = value;
    text.title = title ?? value;
    line.append(label, text);
    lines.append(line);
  };
  if (view.first_utc) {
    said(t("Since"), sinceDate.format(new Date(view.first_utc * 1000)));
  }
  said(
    t("They wrote"),
    view.last_from_them_utc ? listDate(view.last_from_them_utc) : "",
    view.last_from_them_utc ? formatDate(view.last_from_them_utc) : "",
  );
  said(
    t("You wrote"),
    view.last_to_them_utc ? listDate(view.last_to_them_utc) : t("never"),
    view.last_to_them_utc ? formatDate(view.last_to_them_utc) : t("never"),
  );
  if (view.category) {
    said(
      t("Usually"),
      view.bulk
        ? t("{category} — bulk mail", { category: t(view.category) })
        : t(view.category),
    );
  }
  if (view.folders.length) {
    const where = view.folders.map((f) => f.name).join(", ");
    said(t("Filed in"), where, view.folders.map((f) => `${f.name} (${f.messages})`).join(", "));
  }
  if (lines.childElementCount) card.append(lines);

  // The same verdict the reading pane shows for one message, here for the
  // most pressing of theirs still in the Inbox.
  if (view.waiting) {
    const waiting = document.createElement("div");
    waiting.className = "sender-waiting";
    waiting.textContent = t("Waiting on you {when}: {reason}", {
      when: t(LEVELS[view.waiting.score]),
      reason: view.waiting.reason,
    });
    card.append(waiting);
  }

  if (view.recent.length) {
    const list = document.createElement("div");
    list.className = "sender-recent";
    const heading = document.createElement("div");
    heading.className = "sender-recent-head";
    heading.textContent = t("Last correspondence");
    list.append(heading);
    for (const message of view.recent.slice(0, recent)) {
      list.append(recentRow(message, interactive && open, open));
    }
    card.append(list);
  }

  if (interactive && view.address) {
    const actions = document.createElement("div");
    actions.className = "sender-actions";
    const conversation = document.createElement("button");
    conversation.type = "button";
    conversation.textContent = t("Whole conversation");
    conversation.title = t("Everything with this person, as a messenger shows it");
    conversation.onclick = async () => {
      await showView("people");
      await openConversation({ key: view.address });
    };
    actions.append(conversation);
    card.append(actions);
  }

  return card;
}

/// One message of the exchange: which way it went, what it was about, when.
function recentRow(message, interactive, open) {
  const row = document.createElement(interactive ? "button" : "div");
  row.className = `sender-msg${message.unread ? " unread" : ""}`;
  if (interactive) row.type = "button";
  const way = document.createElement("span");
  way.className = "way";
  // An arrow rather than a word: it has to fit beside the subject, and it
  // means the same in every language the window is read in.
  way.textContent = message.from_me ? "→" : "←";
  const subject = document.createElement("span");
  subject.className = "subject";
  subject.textContent = message.subject || message.snippet || t("(no subject)");
  const when = document.createElement("span");
  when.className = "when";
  when.textContent = listDate(message.date_utc);
  row.append(way, subject, when);
  row.title = [
    message.from_me ? t("from you") : t("from them"),
    formatDate(message.date_utc),
    message.snippet ?? "",
  ]
    .filter(Boolean)
    .join("  ·  ");
  if (interactive) row.onclick = () => open(message.id);
  return row;
}

// -- the card in the sidebar -------------------------------------------------

/// Shows the card for the message that has just been opened. Quiet on
/// failure: the message opened, and a card that could not be built is not a
/// reason to put an error over it.
async function showSender({ id = null, address = null, postbox = null }) {
  const asked = `${postbox}|${id}|${address}`;
  senderPanel.dataset.asked = asked;
  let view;
  try {
    view = await senderSummary({ id, address, postbox });
  } catch (err) {
    invoke("log_ui", { level: "warn", message: `sender card: ${err}` }).catch(() => {});
    hideSender();
    return;
  }
  // Another message was opened while this was in flight.
  if (!view || senderPanel.dataset.asked !== asked) return;
  senderPanel.textContent = "";
  senderPanel.append(
    senderCard(view, {
      interactive: true,
      // A letter of theirs opens in the postbox it was scanned into.
      open: postbox ? (letter) => openLetter(postbox, letter) : openWholeMessage,
    }),
  );
  senderPanel.hidden = false;
  syncAside();
}

/// One of their other letters, from the card. The list is already showing
/// this postbox, so it is the cursor that moves.
async function openLetter(postbox, documentId) {
  if (state.postbox?.id !== postbox) return;
  for (const [index, row] of state.rows) {
    if (row.id === documentId) {
      select(index);
      return;
    }
  }
  say(t("that letter is further down the list than has been loaded"));
}

function hideSender() {
  senderPanel.hidden = true;
  senderPanel.textContent = "";
  delete senderPanel.dataset.asked;
  syncAside();
}

// -- the card on hover ---------------------------------------------------------

/// What the row under the cursor is about, or `null` when it is something the
/// core cannot summarise — a scanned letter, or a row still in flight.
function hoverTarget(element) {
  if (state.account === null && !state.postbox) return null;
  const node = element?.closest?.(".row");
  if (!node || node.hidden || node.classList.contains("pending")) return null;
  const row = state.rows.get(Number(node.dataset.index));
  if (!row) return null;
  if (state.postbox) return { node, id: row.id, postbox: state.postbox.id };
  return state.view === "people" ? { node, address: row.key } : { node, id: row.id };
}

viewport.addEventListener("mousemove", (event) => {
  const target = hoverTarget(event.target);
  if (!target) {
    hideHover();
    return;
  }
  const key = hoverKey(target);
  if (key === hoverFor) return;
  hideHover();
  hoverFor = key;
  hoverTimer = setTimeout(() => showHover(target), HOVER_DELAY_MS);
});

viewport.addEventListener("mouseleave", hideHover);
viewport.addEventListener("scroll", hideHover, { passive: true });
window.addEventListener("blur", hideHover);
// Reaching for the keyboard means the cursor has been left where it lies.
document.addEventListener("keydown", hideHover, true);

async function showHover(target) {
  const key = hoverKey(target);
  let view;
  try {
    view = await senderSummary({
      id: target.id ?? null,
      address: target.address ?? null,
      postbox: target.postbox ?? null,
    });
  } catch {
    // Nothing was asked for out loud, so nothing is said out loud.
    return;
  }
  if (!view || hoverFor !== key) return;
  senderHover.textContent = "";
  senderHover.append(senderCard(view, { recent: 3 }));
  senderHover.hidden = false;
  placeHover(target.node);
}

/// What the hover card is about, so moving within one row does not refetch.
function hoverKey(target) {
  return target.postbox ? `post:${target.postbox}:${target.id}` : (target.address ?? target.id);
}

/// Beside the row, on the reading pane's side of the list where there is
/// room, and never off the window.
function placeHover(node) {
  const row = node.getBoundingClientRect();
  const list = el("list").getBoundingClientRect();
  const card = senderHover.getBoundingClientRect();
  const margin = 8;
  let left = list.right + margin;
  if (left + card.width > window.innerWidth - margin) {
    left = Math.max(margin, list.left - card.width - margin);
  }
  const top = Math.min(
    Math.max(margin, row.top),
    Math.max(margin, window.innerHeight - card.height - margin),
  );
  senderHover.style.left = `${Math.round(left)}px`;
  senderHover.style.top = `${Math.round(top)}px`;
}

function hideHover() {
  clearTimeout(hoverTimer);
  hoverTimer = null;
  hoverFor = null;
  senderHover.hidden = true;
  senderHover.textContent = "";
}
