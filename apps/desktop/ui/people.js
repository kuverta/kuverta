// Two other ways of looking at the same mail.
//
// **Needs attention** ranks Inbox mail by how soon it needs you, as the
// urgency agent in core-rpc judges it: the rules at once, the model chosen for
// sorting mail after that. Each row says why.
//
// **People** lists the people you have mail with, and opens the exchange with
// one of them as a messenger does: their messages on the left, yours on the
// right, each showing what it added rather than the whole quoted history.

const conversationPane = el("conversation");
const bubbles = el("bubbles");
const replyBox = el("conversation-reply");

/// The ranked list, fetched once per visit and paged out of memory: it is at
/// most a few hundred rows.
let attention = [];
/// The conversation on screen.
const thread = { key: null, replyTo: null, subject: "" };
/// Whether an urgency pass is running, so a second is not started.
let judging = false;

const ACTIONS = { reply: "reply", pay: "pay", attend: "attend", decide: "decide", read: "read" };
const LEVELS = { 3: "today", 2: "this week", 1: "can wait", 0: "nothing to do" };

ICONS.attention = "M10 3a5 5 0 00-5 5v3.2L3.6 14h12.8L15 11.2V8a5 5 0 00-5-5zM8.3 16.5a1.8 1.8 0 003.4 0";
ICONS.people = "M7.5 9.5a2.8 2.8 0 100-5.6 2.8 2.8 0 000 5.6zM2.5 16.5c.6-2.7 2.6-4.2 5-4.2s4.4 1.5 5 4.2M13.5 9a2.3 2.3 0 100-4.6M14.5 12.4c1.6.4 2.7 1.8 3 4.1";

function askModelForUrgency() {
  try {
    return localStorage.getItem("urgencyModel") !== "no";
  } catch {
    return true;
  }
}

// -- the sidebar entry ----------------------------------------------------------

/// "Needs attention", above the mailboxes, with how many need you this week.
async function renderAttentionNav() {
  const nav = el("attention-nav");
  nav.textContent = "";
  if (state.postbox || state.account === null) return;
  let count = null;
  try {
    count = (await invoke("urgent_messages", { account: state.account })).length;
  } catch {
    count = null;
  }
  nav.append(
    navItem({
      label: "Needs attention",
      icon: "attention",
      unread: count || 0,
      title: "Inbox mail ranked by how soon it needs you, with the reason",
      active: state.view === "urgent",
      onClick: () => showView(state.view === "urgent" ? "mail" : "urgent"),
    }),
  );
}

/// Switches between the ordinary list, Needs attention and People.
async function showView(view) {
  state.view = view;
  if (view !== "mail") state.filter = { ...NO_FILTER };
  hideConversation();
  closeCompose();
  await reload();
}

// -- pages for the list -----------------------------------------------------------

/// A page of the current view in the shape the list draws, for `loadPage`.
async function viewPage(offset, limit) {
  if (state.view === "urgent") {
    // Asked again on every reload, so what was archived or answered leaves.
    if (offset === 0) attention = await invoke("urgent_messages", { account: state.account });
    const slice = attention.slice(offset, offset + limit).map(({ row, urgency }) => ({
      ...row,
      // The reason is the line worth reading here, and the level and action
      // go where the category usually is.
      snippet: urgency.reason,
      category: [LEVELS[urgency.score], ACTIONS[urgency.action], urgency.deadline ? `by ${shortDay(urgency.deadline)}` : ""]
        .filter(Boolean)
        .join(" · "),
    }));
    return { total: attention.length, offset, rows: slice };
  }
  const page = await invoke("conversations", {
    account: state.account,
    offset,
    limit,
    includeBulk: showBulkPeople(),
    query: searchBox.value.trim() || null,
  });
  return {
    total: page.total,
    offset: page.offset,
    rows: page.rows.map((c) => ({
      id: c.key,
      key: c.key,
      from: c.name,
      subject: c.last_subject ?? "",
      snippet: `${c.last_from_me ? "You: " : ""}${c.last_snippet ?? ""}`,
      date_utc: c.last_utc,
      unread: c.unread > 0,
      has_attachments: false,
      category: c.unread ? `${c.unread} new · ${c.messages}` : String(c.messages),
    })),
  };
}

function shortDay(iso) {
  const [y, m, d] = iso.split("-").map(Number);
  return dayMonth.format(new Date(y, m - 1, d));
}

function showBulkPeople() {
  try {
    return localStorage.getItem("peopleBulk") === "yes";
  } catch {
    return false;
  }
}

// -- the scope bar ------------------------------------------------------------------

/// Messages | People, for mail. Drawn by `renderScope`.
function viewToggle() {
  const toggle = document.createElement("span");
  toggle.className = "order view-toggle";
  for (const [value, text, title] of [
    ["mail", "Messages", "Every message, newest first"],
    ["people", "People", "One row per person; open one to see your exchange as a conversation"],
  ]) {
    const button = document.createElement("button");
    button.textContent = text;
    button.title = title;
    if ((state.view === "people") === (value === "people")) button.className = "active";
    button.onclick = () => {
      if ((state.view === "people") !== (value === "people")) showView(value);
    };
    toggle.append(button);
  }
  return toggle;
}

/// What sits in the scope bar in the two views: a way to run the agent again,
/// and whether newsletters count as people.
function viewTools() {
  const tools = [];
  if (state.view === "urgent") {
    const run = document.createElement("button");
    run.textContent = judging ? "Asking…" : "Ask the model";
    run.title = "Let the model chosen for sorting mail judge the mail the rules have judged so far";
    run.disabled = judging;
    run.onclick = () => judgeUrgency({ useModel: true, loud: true });
    tools.push(run);
  }
  if (state.view === "people") {
    const bulk = document.createElement("button");
    bulk.textContent = showBulkPeople() ? "hide bulk senders" : "show bulk senders";
    bulk.title = "Newsletters, marketing and notifications are left out unless shown";
    bulk.onclick = async () => {
      try {
        localStorage.setItem("peopleBulk", showBulkPeople() ? "no" : "yes");
      } catch {
        // Lasts until the window closes.
      }
      await reload();
    };
    tools.push(bulk);
  }
  return tools;
}

// -- the urgency agent ---------------------------------------------------------------

/// Runs the agent: the rules for everything new, then the model when allowed.
/// Quiet unless `loud`, since it also runs by itself after each sync.
async function judgeUrgency({ useModel = askModelForUrgency(), loud = false } = {}) {
  if (judging || state.account === null || state.postbox) return;
  judging = true;
  const account = state.account;
  renderScope();
  try {
    const channel = new window.__TAURI__.core.Channel();
    channel.onmessage = (at) => {
      if (state.account !== account || !at.total || at.done >= at.total) return;
      statusBar.textContent = `${state.email} — judging urgency ${at.done + 1} of ${at.total}`;
    };
    const pass = await invoke("find_urgent", { account, useModel, onProgress: channel });
    if (loud || pass.stopped) {
      const parts = [];
      if (pass.by_model) parts.push(`${pass.model} judged ${pass.by_model}`);
      if (pass.by_rules) parts.push(`the rules judged ${pass.by_rules}`);
      if (!parts.length) parts.push("nothing new to judge");
      say(pass.stopped ? `${parts.join(", ")}; the model stopped: ${pass.stopped}` : parts.join(", "), Boolean(pass.stopped));
    }
  } catch (err) {
    if (loud) say(String(err), true);
  } finally {
    judging = false;
    if (state.email) statusBar.textContent = state.email;
  }
  if (state.account !== account) return;
  if (state.view === "urgent") {
    attention = await invoke("urgent_messages", { account }).catch(() => attention);
    await reload({ keepPosition: true });
  } else {
    await renderAttentionNav();
    renderScope();
  }
}

/// The verdict on the open message, under its subject.
async function showUrgency(id) {
  const note = el("reading-urgency");
  note.hidden = true;
  let verdict = null;
  try {
    verdict = await invoke("urgency_of", { id });
  } catch {
    return;
  }
  if (!verdict || verdict.score < 2) return;
  const who = verdict.source === "model" ? `judged by ${verdict.model}` : "judged by the rules";
  const due = verdict.deadline ? ` · by ${shortDay(verdict.deadline)}` : "";
  const action = ACTIONS[verdict.action] ? ` · ${verdict.action}` : "";
  note.textContent = `Needs you ${LEVELS[verdict.score]}${action}${due}: ${verdict.reason} (${who})`;
  note.hidden = false;
}

// -- a conversation ---------------------------------------------------------------------

const bubbleTime = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" });
const bubbleDay = new Intl.DateTimeFormat(undefined, { weekday: "long", day: "numeric", month: "long", year: "numeric" });

function hideConversation() {
  conversationPane.hidden = true;
  thread.key = null;
}

function initials(name) {
  const words = name.replace(/<.*>/, "").replace(/[^\p{L}\s]/gu, " ").trim().split(/\s+/);
  return ((words[0]?.[0] ?? "?") + (words.length > 1 ? words[words.length - 1][0] : "")).toUpperCase();
}

async function openConversation(row) {
  let data;
  try {
    data = await invoke("conversation", { account: state.account, key: row.key });
  } catch (err) {
    say(`could not open the conversation: ${err}`, true);
    return;
  }
  const keepDraft = thread.key === row.key;
  thread.key = row.key;
  thread.replyTo = data.reply_to;
  thread.subject = [...data.bubbles].reverse().find((b) => b.subject)?.subject ?? "";

  reading.hidden = true;
  compose.pane.hidden = true;
  emptyPane.hidden = true;
  conversationPane.hidden = false;
  el("conversation-avatar").textContent = initials(data.name);
  el("conversation-name").textContent = data.name;
  el("conversation-address").textContent = data.key;
  if (!keepDraft) replyBox.value = "";

  bubbles.textContent = "";
  let lastDay = "";
  let lastSubject = "";
  for (const bubble of data.bubbles) {
    const when = bubble.date_utc ? new Date(bubble.date_utc * 1000) : null;
    const day = when ? bubbleDay.format(when) : "";
    if (day && day !== lastDay) {
      const divider = document.createElement("div");
      divider.className = "bubble-day";
      divider.textContent = day;
      bubbles.append(divider);
      lastDay = day;
    }
    const subject = (bubble.subject ?? "").replace(/^((re|aw|fwd?|wg):\s*)+/i, "");
    bubbles.append(makeBubble(bubble, subject && subject !== lastSubject ? subject : null, when));
    if (subject) lastSubject = subject;
  }
  if (!data.bubbles.length) {
    const none = document.createElement("div");
    none.className = "bubble-day";
    none.textContent = "No messages to show.";
    bubbles.append(none);
  }
  bubbles.scrollTop = bubbles.scrollHeight;
}

function makeBubble(bubble, subject, when) {
  const wrap = document.createElement("div");
  wrap.className = `bubble-row ${bubble.from_me ? "mine" : "theirs"}`;
  const body = document.createElement("div");
  body.className = `bubble${bubble.unread ? " unread" : ""}`;
  if (subject) {
    const head = document.createElement("div");
    head.className = "bubble-subject";
    head.textContent = subject;
    body.append(head);
  }
  const text = document.createElement("div");
  text.className = "bubble-text";
  // textContent, never innerHTML: this was written by whoever sent it.
  text.textContent = bubble.text || "(nothing but an attachment)";
  body.append(text);
  const time = document.createElement("div");
  time.className = "bubble-time";
  time.textContent = when ? bubbleTime.format(when) : "";
  body.append(time);
  // Double-click opens the whole message, quotes and all, in the ordinary view.
  body.title = "Double-click for the whole message";
  body.addEventListener("dblclick", () => openWholeMessage(bubble.id));
  wrap.append(body);
  return wrap;
}

async function openWholeMessage(id) {
  await showView("mail");
  try {
    const detail = await invoke("message", { account: state.account, id });
    hideConversation();
    reading.hidden = false;
    emptyPane.hidden = true;
    el("reading-actions").hidden = true;
    el("reading-subject").textContent = detail.subject ?? "(no subject)";
    el("reading-meta").textContent = [detail.from, formatDate(detail.date_utc)].filter(Boolean).join("  ·  ");
    el("reading-body").textContent = detail.body_text ?? "(no readable body)";
    el("reading-urgency").hidden = true;
    showSecurity(detail);
    showAttachments(detail);
  } catch (err) {
    say(String(err), true);
  }
}

/// Sends what is typed under the conversation: a reply to their latest
/// message when there is one, so it threads, else a new message.
async function sendInConversation() {
  const text = replyBox.value.trim();
  if (!text || !thread.key) return;
  const button = el("conversation-send");
  button.disabled = true;
  try {
    const sent = await invoke("send", {
      email: state.email,
      draft: {
        to: [thread.key],
        cc: [],
        bcc: [],
        subject: thread.replyTo ? "" : thread.subject || "Hello",
        body: `${text}\n`,
        reply_to: thread.replyTo,
        reply_all: false,
        forward: null,
        sign: false,
        encrypt: false,
      },
    });
    replyBox.value = "";
    // Shown now; the copy filed in Sent takes its place at the next sync.
    bubbles.append(makeBubble({ id: null, from_me: true, text, unread: false }, null, new Date()));
    bubbles.scrollTop = bubbles.scrollHeight;
    say(sent.filing_error ? `sent — but not filed: ${sent.filing_error}` : "sent");
  } catch (err) {
    say(String(err), true);
  } finally {
    button.disabled = false;
  }
}

el("conversation-send").onclick = sendInConversation;
replyBox.addEventListener("keydown", (event) => {
  if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
    event.preventDefault();
    sendInConversation();
  }
});

// The same preference as the rest, in Settings → General.
{
  const pref = el("pref-urgency-model");
  SETTINGS_PAGES.general.addEventListener("show", () => (pref.checked = askModelForUrgency()));
  pref.addEventListener("change", () => {
    try {
      localStorage.setItem("urgencyModel", pref.checked ? "yes" : "no");
    } catch {
      // Lasts until the window closes.
    }
  });
}
