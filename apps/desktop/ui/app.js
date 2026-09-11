// The triage surface.
//
// The virtualizer is the one from the spike and works the same way: a fixed
// pool of DOM nodes is recycled as the viewport moves, so the node count stays
// proportional to the window rather than to the mailbox. What changed is where
// the rows come from. The spike held all of them in memory; here the list owns
// only the pages it has asked for, because the bridge that made the spike
// worth running is the same bridge a whole mailbox would have to cross.

const { invoke } = window.__TAURI__.core;

const ROW_HEIGHT = 30;
const OVERSCAN = 8;
// One page is several screenfuls, so scrolling at a normal speed stays ahead
// of the fetch without asking for much that is never shown.
const PAGE = 200;

const el = (id) => document.getElementById(id);
const viewport = el("viewport");
const spacer = el("spacer");
const content = el("content");
const statusBar = el("status");
const reading = el("reading");
const filters = el("filters");
const searchBox = el("search");
const toast = el("toast");

const state = {
  account: null,
  email: null,
  syncing: false,
  archive: null,
  trash: null,
  filter: { category: null, unreadOnly: false },
  total: 0,
  // Sparse: offset -> row. Only what has been fetched.
  rows: new Map(),
  // Page offsets already requested, so a slow fetch is not asked for twice.
  requested: new Set(),
  selected: -1,
  searching: false,
  pool: [],
  firstRendered: -1,
};

// -- data ------------------------------------------------------------------

async function loadPage(offset) {
  if (state.requested.has(offset)) return;
  state.requested.add(offset);

  try {
    const page = await invoke("messages", {
      account: state.account,
      offset,
      limit: PAGE,
      category: state.filter.category,
      unreadOnly: state.filter.unreadOnly,
    });

    // The total can move under us while a sync is running, so it is taken
    // from every page rather than once at the start.
    if (page.total !== state.total) {
      state.total = page.total;
      spacer.style.height = `${state.total * ROW_HEIGHT}px`;
    }
    page.rows.forEach((row, i) => state.rows.set(page.offset + i, row));
    render(true);
  } catch (err) {
    state.requested.delete(offset);
    say(`could not load messages: ${err}`, true);
  }
}

/** Throws away everything fetched, e.g. after a filter change. */
async function reload() {
  state.rows.clear();
  state.requested.clear();
  state.firstRendered = -1;
  state.selected = -1;
  state.searching = false;
  reading.hidden = true;
  viewport.scrollTop = 0;
  await loadPage(0);
  await refreshFilters();
}

async function refreshFilters() {
  const counts = await invoke("category_counts", { account: state.account });
  filters.textContent = "";

  const add = (label, count, active, onClick) => {
    const button = document.createElement("button");
    button.className = active ? "filter active" : "filter";
    button.textContent = count === null ? label : `${label} ${count}`;
    button.onclick = onClick;
    filters.append(button);
  };

  add("all", state.total, !state.filter.category && !state.filter.unreadOnly, async () => {
    state.filter = { category: null, unreadOnly: false };
    await reload();
  });
  add("unread", null, state.filter.unreadOnly, async () => {
    state.filter = { ...state.filter, unreadOnly: !state.filter.unreadOnly };
    await reload();
  });
  for (const [category, count] of counts) {
    add(category, count, state.filter.category === category, async () => {
      const next = state.filter.category === category ? null : category;
      state.filter = { ...state.filter, category: next };
      await reload();
    });
  }
}

// -- rendering -------------------------------------------------------------

function buildPool() {
  const visible = Math.ceil(viewport.clientHeight / ROW_HEIGHT);
  const size = visible + OVERSCAN * 2;
  if (state.pool.length === size) return;

  content.textContent = "";
  state.pool = [];
  for (let i = 0; i < size; i++) {
    const row = document.createElement("div");
    row.className = "row";

    const unread = document.createElement("span");
    unread.className = "dot";
    const date = document.createElement("span");
    date.className = "date";
    const sender = document.createElement("span");
    sender.className = "sender";
    const subject = document.createElement("span");
    subject.className = "subject";
    const tag = document.createElement("span");
    tag.className = "tag";

    row.append(unread, date, sender, subject, tag);
    content.append(row);
    state.pool.push({ row, unread, date, sender, subject, tag });
  }
  state.firstRendered = -1;
}

const dateFormat = new Intl.DateTimeFormat(undefined, {
  year: "numeric",
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit",
});

function formatDate(seconds) {
  if (!seconds) return "";
  return dateFormat.format(new Date(seconds * 1000));
}

/** Draws the slice of rows at the current scroll offset. */
function render(force = false) {
  const maxFirst = Math.max(0, state.total - state.pool.length);
  const first = Math.max(
    0,
    Math.min(Math.floor(viewport.scrollTop / ROW_HEIGHT) - OVERSCAN, maxFirst),
  );
  if (first === state.firstRendered && !force) return;
  state.firstRendered = first;

  content.style.transform = `translateY(${first * ROW_HEIGHT}px)`;

  for (let i = 0; i < state.pool.length; i++) {
    const index = first + i;
    const node = state.pool[i];
    const row = state.rows.get(index);

    if (index >= state.total) {
      node.row.hidden = true;
      continue;
    }
    node.row.hidden = false;
    node.row.classList.toggle("selected", index === state.selected);

    if (!row) {
      // The page is still in flight. A placeholder keeps the row height
      // correct so the scrollbar does not jump when it lands.
      node.row.classList.add("pending");
      node.unread.className = "dot";
      node.date.textContent = "";
      node.sender.textContent = "";
      node.subject.textContent = "…";
      node.tag.textContent = "";
      continue;
    }

    node.row.classList.remove("pending");
    node.unread.className = row.unread ? "dot on" : "dot";
    node.date.textContent = formatDate(row.date_utc);
    node.sender.textContent = row.from;
    node.subject.textContent = row.subject;
    node.tag.textContent = [row.category, row.has_attachments ? "📎" : ""]
      .filter(Boolean)
      .join(" ");
  }

  ensureLoaded(first, first + state.pool.length);
}

/** Requests any page covering the visible range that is not in hand. */
function ensureLoaded(from, to) {
  if (state.searching) return;
  const firstPage = Math.floor(Math.max(0, from) / PAGE) * PAGE;
  const lastPage = Math.floor(Math.min(to, Math.max(0, state.total - 1)) / PAGE) * PAGE;
  for (let offset = firstPage; offset <= lastPage; offset += PAGE) {
    if (!state.rows.has(offset)) loadPage(offset);
  }
}

// -- reading ---------------------------------------------------------------

async function openSelected() {
  const row = state.rows.get(state.selected);
  if (!row) return;

  try {
    const detail = await invoke("message", { account: state.account, id: row.id });
    reading.hidden = false;
    el("reading-subject").textContent = detail.subject ?? "(no subject)";
    el("reading-meta").textContent = [
      detail.from,
      formatDate(detail.date_utc),
      detail.folders.join(", "),
    ]
      .filter(Boolean)
      .join("  ·  ");
    el("reading-body").textContent =
      detail.body_text ?? "(no plain-text body — HTML rendering is not built yet)";
  } catch (err) {
    say(`could not open: ${err}`, true);
  }
}

function select(index) {
  if (state.total === 0) return;
  state.selected = Math.max(0, Math.min(index, state.total - 1));

  // Keep the selection in view, which is what makes j/k usable at all.
  const top = state.selected * ROW_HEIGHT;
  const bottom = top + ROW_HEIGHT;
  if (top < viewport.scrollTop) viewport.scrollTop = top;
  else if (bottom > viewport.scrollTop + viewport.clientHeight) {
    viewport.scrollTop = bottom - viewport.clientHeight;
  }
  render(true);
  if (!reading.hidden) openSelected();
}

// -- changes ---------------------------------------------------------------

let toastTimer = null;

function say(message, isError = false) {
  toast.textContent = message;
  toast.className = isError ? "toast error" : "toast";
  toast.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    toast.hidden = true;
  }, 4000);
}

/// Applies a queued change and takes the row out of the list straight away.
///
/// The change has not reached the server yet — it is sitting in the undo
/// window — so this is the list agreeing with what was asked for rather than
/// with the store. The next sync reconciles, and `undo` puts the row back.
async function act(command, args, describe) {
  const row = state.rows.get(state.selected);
  if (!row) return;

  try {
    await invoke(command, { account: state.account, id: row.id, ...args });
    say(`${describe} — z to undo`);
    await reload();
  } catch (err) {
    say(String(err), true);
  }
}

async function archive() {
  if (!state.archive) {
    say("this account has no Archive folder", true);
    return;
  }
  await act("move_to", { target: state.archive }, `archived`);
}

async function trash() {
  if (!state.trash) {
    say("this account has no Trash folder", true);
    return;
  }
  await act("move_to", { target: state.trash }, `moved to ${state.trash}`);
}

async function toggleRead() {
  const row = state.rows.get(state.selected);
  if (!row) return;
  await act("set_read", { read: row.unread }, row.unread ? "marked read" : "marked unread");
}

async function undo() {
  try {
    const change = await invoke("undo", { account: state.account });
    say(change ? `undone: ${change.what}` : "nothing to undo");
    if (change) await reload();
  } catch (err) {
    say(String(err), true);
  }
}

// -- compose ---------------------------------------------------------------

const compose = {
  pane: el("compose"),
  what: el("compose-what"),
  envelope: el("compose-envelope"),
  to: el("compose-to"),
  cc: el("compose-cc"),
  bcc: el("compose-bcc"),
  subject: el("compose-subject"),
  body: el("compose-body"),
  send: el("compose-send"),
  cancel: el("compose-cancel"),
  // The message being replied to or forwarded, by row id.
  replyTo: null,
  replyAll: false,
  forward: null,
  sending: false,
};

function addresses(field) {
  return field.value
    .split(",")
    .map((a) => a.trim())
    .filter(Boolean);
}

function draftInput() {
  return {
    to: addresses(compose.to),
    cc: addresses(compose.cc),
    bcc: addresses(compose.bcc),
    subject: compose.subject.value,
    body: compose.body.value,
    reply_to: compose.replyTo,
    reply_all: compose.replyAll,
    forward: compose.forward,
  };
}

function closeCompose() {
  compose.pane.hidden = true;
  compose.replyTo = null;
  compose.forward = null;
  compose.replyAll = false;
  compose.envelope.textContent = "";
  for (const field of [compose.to, compose.cc, compose.bcc, compose.subject, compose.body]) {
    field.value = "";
  }
}

/// Opens compose. With `replyAll`, the recipients and quoted body come from
/// the core rather than being assembled here — reply-all has rules (drop
/// yourself, honour Reply-To) that belong in one place.
async function openCompose({ replyAll = null, forward = false } = {}) {
  const row = state.rows.get(state.selected);
  closeCompose();

  if (replyAll !== null || forward) {
    if (!row) {
      say("select a message first", true);
      return;
    }
    if (forward) {
      compose.forward = row.id;
      compose.what.textContent = "Forward";
    } else {
      compose.replyTo = row.id;
      compose.replyAll = replyAll;
      compose.what.textContent = replyAll ? "Reply all" : "Reply";
    }

    // Ask the core what the draft looks like, so the recipients and the
    // quoting shown are the ones that would actually be sent.
    try {
      const preview = await invoke("preview", {
        email: state.email,
        draft: draftInput(),
      });
      compose.subject.value = preview.subject;
      compose.to.value = preview.recipients.join(", ");
    } catch (err) {
      // A forward has no recipients yet, which the core rejects. That is not
      // an error here — it is the point of the empty To: field.
      compose.subject.value = forward ? `Fwd: ${row.subject}` : `Re: ${row.subject}`;
    }
  } else {
    compose.what.textContent = "New message";
  }

  compose.pane.hidden = false;
  reading.hidden = true;
  (compose.to.value ? compose.body : compose.to).focus();
  await refreshEnvelope();
}

/// Shows who would actually receive it, Bcc included.
///
/// The envelope is the only place a blind recipient appears, and the one thing
/// worth checking before committing — so it is on screen rather than implied.
async function refreshEnvelope() {
  if (compose.pane.hidden) return;
  try {
    const preview = await invoke("preview", {
      email: state.email,
      draft: draftInput(),
    });
    compose.envelope.textContent = `${preview.recipients.length} recipient(s): ${preview.recipients.join(", ")}`;
  } catch (err) {
    compose.envelope.textContent = String(err);
  }
}

async function sendDraft() {
  if (compose.sending) return;
  compose.sending = true;
  compose.send.disabled = true;
  compose.send.textContent = "Sending…";

  try {
    const sent = await invoke("send", { email: state.email, draft: draftInput() });
    if (sent.filing_error) {
      // Sent is sent. Saying it failed would invite sending it twice.
      say(`sent to ${sent.recipients.length} — but not filed: ${sent.filing_error}`, true);
    } else {
      const filed = sent.filed_in ? `, filed in ${sent.filed_in}` : "";
      say(`sent to ${sent.recipients.length} recipient(s)${filed}`);
    }
    closeCompose();
    await reload();
  } catch (err) {
    say(String(err), true);
  } finally {
    compose.sending = false;
    compose.send.disabled = false;
    compose.send.textContent = "Send";
  }
}

compose.send.onclick = sendDraft;
compose.cancel.onclick = closeCompose;
for (const field of [compose.to, compose.cc, compose.bcc]) {
  field.addEventListener("change", refreshEnvelope);
}

// -- sync ------------------------------------------------------------------

/// Sends queued changes and fetches new mail.
///
/// The button is disabled while it runs rather than queueing a second pass:
/// two syncs of one account racing each other is a way to discover locking
/// behaviour, not a feature.
async function sync() {
  if (state.syncing) return;
  state.syncing = true;
  statusBar.textContent = `${state.email} — syncing…`;

  try {
    const s = await invoke("sync", { email: state.email });
    const parts = [];
    if (s.changes_sent) parts.push(`${s.changes_sent} change(s) sent`);
    if (s.changes_refused) parts.push(`${s.changes_refused} refused`);
    if (s.inserted) parts.push(`${s.inserted} new`);
    if (s.expunged) parts.push(`${s.expunged} gone`);
    say(parts.length ? parts.join(", ") : "nothing new");
    await reload();
  } catch (err) {
    say(String(err), true);
  } finally {
    state.syncing = false;
    statusBar.textContent = state.email;
  }
}

// -- search ----------------------------------------------------------------

async function runSearch(query) {
  if (!query.trim()) {
    await reload();
    return;
  }
  try {
    const rows = await invoke("search", {
      account: state.account,
      query,
      limit: 200,
    });
    state.rows.clear();
    state.requested.clear();
    state.searching = true;
    rows.forEach((row, i) => state.rows.set(i, row));
    state.total = rows.length;
    state.selected = rows.length ? 0 : -1;
    spacer.style.height = `${state.total * ROW_HEIGHT}px`;
    viewport.scrollTop = 0;
    render(true);
    say(`${rows.length} result(s) — Escape to go back`);
  } catch (err) {
    say(String(err), true);
  }
}

// -- keys ------------------------------------------------------------------

const KEYS = {
  j: () => select(state.selected + 1),
  ArrowDown: () => select(state.selected + 1),
  k: () => select(state.selected - 1),
  ArrowUp: () => select(state.selected - 1),
  Enter: openSelected,
  o: openSelected,
  e: archive,
  "#": trash,
  Delete: trash,
  u: toggleRead,
  z: undo,
  r: sync,
  c: () => openCompose(),
  R: () => openCompose({ replyAll: false }),
  A: () => openCompose({ replyAll: true }),
  f: () => openCompose({ forward: true }),
};

document.addEventListener("keydown", async (event) => {
  // While composing, the keys belong to the fields — otherwise typing "e"
  // into a subject line would archive something.
  if (compose.pane.contains(event.target)) {
    if (event.key === "Escape") closeCompose();
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) await sendDraft();
    return;
  }

  if (event.target === searchBox) {
    if (event.key === "Enter") await runSearch(searchBox.value);
    if (event.key === "Escape") {
      searchBox.value = "";
      searchBox.blur();
      await reload();
    }
    return;
  }

  if (event.key === "/") {
    event.preventDefault();
    searchBox.focus();
    return;
  }
  if (event.key === "Escape") {
    reading.hidden = true;
    return;
  }

  const action = KEYS[event.key];
  if (action) {
    event.preventDefault();
    await action();
  }
});

viewport.addEventListener("scroll", () => render(), { passive: true });
window.addEventListener("resize", () => {
  buildPool();
  render(true);
});

content.addEventListener("click", (event) => {
  const index = state.pool.findIndex((node) => node.row === event.target.closest(".row"));
  if (index >= 0) {
    select(state.firstRendered + index);
    openSelected();
  }
});

// -- start -----------------------------------------------------------------

async function start() {
  const accounts = await invoke("accounts");
  if (!accounts.length) {
    statusBar.textContent = "no accounts — run `fuckmail add-account` first";
    return;
  }

  state.account = accounts[0].id;
  state.email = accounts[0].email;
  statusBar.textContent = state.email;

  const special = await invoke("special_folders", { account: state.account });
  state.archive = special.archive;
  state.trash = special.trash;

  buildPool();
  await reload();
  select(0);

  const pending = await invoke("queue", { account: state.account });
  if (pending.length) say(`${pending.length} change(s) waiting for the next sync`);
}

start().catch((err) => {
  statusBar.textContent = `failed to start: ${err}`;
});
