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
// A stopped background model is reported once, then retried after a pause.
// Rules still run; an explicit request always retries immediately.
let urgencyRetryAt = 0;
let urgencyFailure = null;

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
  if (state.postbox || state.account === null) {
    nav.replaceChildren();
    return;
  }
  const account = state.account;
  let count = null;
  try {
    count = (await invoke("urgent_messages", { account })).length;
  } catch {
    count = null;
  }
  if (state.postbox || state.account !== account) return;
  // Keep the current entry visible while the next count is loading.
  nav.replaceChildren(
    navItem({
      label: t("Needs attention"),
      icon: "attention",
      unread: count || 0,
      title: t("Inbox mail ranked by how soon it needs you, with the reason"),
      active: state.view === "urgent",
      onClick: () => showView(state.view === "urgent" ? "mail" : "urgent"),
    }),
  );
}

/// Switches between the ordinary list, Needs attention and People.
async function showView(view) {
  state.view = view;
  if (view !== "mail") state.filter = { ...NO_FILTER, unreadOnly: state.filter.unreadOnly };
  hideConversation();
  closeCompose();
  await reload();
}

// -- pages for the list -----------------------------------------------------------

/// A page of the current view in the shape the list draws, for `loadPage`.
async function viewPage(offset, limit) {
  if (state.view === "urgent") {
    // Asked again on every reload, so what was archived or answered leaves.
    if (offset === 0) {
      attention = await invoke("urgent_messages", { account: state.account });
      if (state.filter.unreadOnly) attention = attention.filter(({ row }) => row.unread);
    }
    const slice = attention.slice(offset, offset + limit).map(({ row, urgency }) => ({
      ...row,
      // The reason is the line worth reading here, and the level and action
      // go where the category usually is.
      snippet: urgency.reason,
      category: [t(LEVELS[urgency.score]), ACTIONS[urgency.action] ? t(ACTIONS[urgency.action]) : "", urgency.deadline ? t("by {day}", { day: shortDay(urgency.deadline) }) : ""]
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
    unreadOnly: state.filter.unreadOnly,
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
      snippet: c.last_from_me ? t("You: {text}", { text: c.last_snippet ?? "" }) : c.last_snippet ?? "",
      date_utc: c.last_utc,
      unread: c.unread > 0,
      has_attachments: false,
      category: c.unread ? t("{unread} new · {total}", { unread: c.unread, total: c.messages }) : String(c.messages),
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
    button.textContent = t(text);
    button.title = t(title);
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
    const criteria = document.createElement("button");
    criteria.textContent = t("Criteria…");
    criteria.title = t("Recent Inbox mail only — configure keywords in General settings");
    criteria.onclick = () => openSettings("general").then(() => el("attention-account").scrollIntoView({block:"center"}));
    tools.push(criteria);
    const run = document.createElement("button");
    run.textContent = judging ? t("Asking…") : t("Ask the model");
    run.title = t("Let the model chosen for sorting mail judge the mail the rules have judged so far");
    run.disabled = judging;
    run.onclick = () => judgeUrgency({ useModel: true, loud: true });
    tools.push(run);
  }
  if (state.view === "people") {
    const bulk = document.createElement("button");
    bulk.textContent = showBulkPeople() ? t("hide bulk senders") : t("show bulk senders");
    bulk.title = t("Newsletters, marketing and notifications are left out unless shown");
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
  const attemptedModel = useModel && (loud || Date.now() >= urgencyRetryAt);
  renderScope();
  try {
    const channel = new window.__TAURI__.core.Channel();
    channel.onmessage = (at) => {
      if (state.account !== account || !at.total || at.done >= at.total) return;
      statusBar.textContent = t("{email} — judging urgency {done} of {total}", {
        email: state.email,
        done: at.done + 1,
        total: at.total,
      });
    };
    const pass = await invoke("find_urgent", { account, useModel: attemptedModel, onProgress: channel });
    const newFailure = pass.stopped && pass.stopped !== urgencyFailure;
    if (pass.stopped) {
      urgencyFailure = pass.stopped;
      urgencyRetryAt = Date.now() + 30 * 60 * 1000;
    } else if (attemptedModel) {
      urgencyFailure = null;
      urgencyRetryAt = 0;
    }
    if (loud || newFailure) {
      const parts = [];
      if (pass.by_model) parts.push(t("{model} judged {count}", { model: pass.model, count: pass.by_model }));
      if (pass.by_rules) parts.push(t("the rules judged {count}", { count: pass.by_rules }));
      if (!parts.length) parts.push(t("nothing new to judge"));
      say(
        pass.stopped
          ? t("{what}; the model stopped: {why}", { what: parts.join(", "), why: pass.stopped })
          : parts.join(", "),
        Boolean(pass.stopped),
      );
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
  const who = verdict.source === "model" ? t("judged by {model}", { model: verdict.model }) : t("judged by the rules");
  const due = verdict.deadline ? ` · ${t("by {day}", { day: shortDay(verdict.deadline) })}` : "";
  const action = ACTIONS[verdict.action] ? ` · ${t(verdict.action)}` : "";
  note.textContent = t("Needs you {when}{action}{due}: {reason} ({who})", {
    when: t(LEVELS[verdict.score]),
    action,
    due,
    reason: verdict.reason,
    who,
  });
  note.hidden = false;
}

// -- a conversation ---------------------------------------------------------------------

const bubbleTime = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" });
const bubbleDay = new Intl.DateTimeFormat(undefined, { weekday: "long", day: "numeric", month: "long", year: "numeric" });

function hideConversation() {
  clearThreadMailViews();
  conversationPane.hidden = true;
  el("mail-thread").hidden = true;
  thread.key = null;
}

// A subject/reply thread is distinct from People: it may have many participants.
// Keep one group in the Inbox list; show its individual messages in the reader.
let mailThreadTicket = 0;
async function openMailThread(group, { autoOpen = false } = {}) {
  const ticket = ++mailThreadTicket;
  const generation = listGeneration;
  const account = accountOf(group);
  const current = () => ticket === mailThreadTicket && generation === listGeneration &&
    state.rows.get(state.selected)?.id === group.id && threadOverview();
  hideConversation();
  reading.hidden = true;
  emptyPane.hidden = true;
  const pane = el("mail-thread");
  pane.hidden = false;
  el("mail-thread-title").textContent = group.subject;
  el("mail-thread-count").textContent = "";
  const messages = el("mail-thread-messages");
  messages.replaceChildren();
  const more = el("mail-thread-more");
  let offset = 0;
  let openedLatest = false;
  let unreadTicket = 0;
  async function refreshGroupUnread() {
    const latest = ++unreadTicket;
    try {
      // Do not clear the group dot while an unread reply remains on a later
      // page. Ask only for the remaining count, without reading any bodies.
      const remaining = await invoke("messages", {account, offset:0, limit:1,
        filter:{threadId:group.thread_id, threaded:false, unreadOnly:true}});
      if (current() && latest === unreadTicket) {
        group.unread = remaining.total > 0;
        render(true);
      }
    } catch (_) { /* The mail was read successfully; keep the last group state. */ }
  }
  async function loadMore() {
    more.disabled = true;
    try {
      const page = await invoke("messages", {account, offset, limit:50,
        filter:{threadId:group.thread_id, threaded:false, unreadOnly:false}});
      if (!current()) return;
      for (const row of page.rows) {
        row.account = account;
        const entry = document.createElement("details");
        entry.className = "mail-thread-entry";
        const summary = document.createElement("summary");
        const head = document.createElement("strong");
        const meta = document.createElement("span");
        const preview = document.createElement("span");
        const updateHeader = () => { head.textContent = `${row.unread ? "● " : ""}${row.from}`; };
        updateHeader();
        meta.className = "hint";
        meta.textContent = [formatDate(row.date_utc), row.to ? t("Sent") : ""].filter(Boolean).join(" · ");
        preview.className = "mail-thread-preview hint";
        preview.textContent = row.snippet || row.subject;
        summary.append(head, meta, preview);
        const body = document.createElement("div");
        body.className = "mail-thread-body";
        const open = document.createElement("button");
        open.textContent = t("Open individual message");
        open.onclick = async () => {
          if (!(await leaveCompose()) || !current()) return;
          threadBack = {...state.filter};
          state.filter = {...NO_FILTER, thread:group.thread_id};
          await reload();
          let index = indexOfRow(row.id);
          for (let at = PAGE; index < 0 && at < state.total; at += PAGE) {
            await loadPage(at);
            index = indexOfRow(row.id);
          }
          if (index >= 0) {
            select(index,{open:false});
            await openSelected({markRead:true});
          }
        };
        let loaded = false;
        let loading = false;
        entry.ontoggle = async () => {
          if (!entry.open || loading || !current()) return;
          loading = true;
          try {
            if (!loaded) {
              const detail = await invoke("message",{account,id:row.id});
              if (!current()) return;
              meta.textContent = `${formatDate(detail.date_utc)} · ${detail.folders.join(", ")}`;
              row.from = detail.from;
              body.replaceChildren(threadMailView(detail, account));
              loaded = true;
            }
            if (entry.open) {
              const wasUnread = row.unread;
              await markOpenedRead(row);
              if (wasUnread && !row.unread && current()) await refreshGroupUnread();
            }
          } catch (err) { body.textContent = String(err); }
          finally { loading = false; updateHeader(); }
        };
        entry.append(summary, body, open);
        messages.append(entry);
        if (autoOpen && !openedLatest && el("pref-thread-open-latest").checked) {
          openedLatest = true;
          entry.open = true;
        }
      }
      offset += page.rows.length;
      el("mail-thread-count").textContent = t("{shown} of {total} messages",{shown:offset,total:page.total});
      more.hidden = offset >= page.total || !page.rows.length;
    } catch (err) { if (current()) el("mail-thread-count").textContent = String(err); }
    finally { if (current()) more.disabled = false; }
  }
  more.hidden = true;
  more.onclick = loadMore;
  await loadMore();
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
    say(t("could not open the conversation: {error}", { error: err }), true);
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
  const name = el("conversation-name");
  name.textContent = "";
  name.append(senderName(data.name, { address: data.key }, { className: "sender-link plain" }));
  el("conversation-address").textContent = data.key;
  showSender({ address: data.key });
  if (!keepDraft) {
    replyBox.value = "";
    conversationFiles.clear();
  }

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
    none.textContent = t("No messages to show.");
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
  text.textContent = bubble.text || t("(nothing but an attachment)");
  body.append(text);
  const time = document.createElement("div");
  time.className = "bubble-time";
  time.textContent = when ? bubbleTime.format(when) : "";
  body.append(time);
  // Double-click opens the whole message, quotes and all, in the ordinary view.
  body.title = t("Double-click for the whole message");
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
    el("reading-subject").textContent = detail.subject ?? t("(no subject)");
    fillMeta(el("reading-meta"), detail.from, { id }, [formatDate(detail.date_utc)]);
    el("reading-body").textContent = detail.body_text ?? t("(no readable body)");
    el("reading-urgency").hidden = true;
    showSender({ id });
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
  const files = conversationFiles.forDraft();
  if ((!text && !files.length) || !thread.key) return;
  const button = el("conversation-send");
  button.disabled = true;
  try {
    const sent = await invoke("send", {
      email: state.email,
      draft: {
        to: [thread.key],
        cc: [],
        bcc: [],
        subject: thread.replyTo ? "" : thread.subject || t("Hello"),
        body: `${text}\n`,
        reply_to: thread.replyTo,
        reply_all: false,
        forward: null,
        sign: false,
        encrypt: false,
        attachments: files,
      },
    });
    replyBox.value = "";
    conversationFiles.clear();
    // Shown now; the copy filed in Sent takes its place at the next sync.
    const names = files.map((file) => `📎 ${file.name}`).join("\n");
    bubbles.append(
      makeBubble({ id: null, from_me: true, text: [text, names].filter(Boolean).join("\n\n"), unread: false }, null, new Date()),
    );
    bubbles.scrollTop = bubbles.scrollHeight;
    say(sent.filing_error ? t("sent — but not filed: {error}", { error: sent.filing_error }) : t("sent"));
  } catch (err) {
    say(String(err), true);
  } finally {
    button.disabled = false;
  }
}

el("conversation-send").onclick = sendInConversation;

// Criteria belong to an account, not the currently selected window/profile.
const attentionForm = { ticket: 0, account: null, defaults: null };
const attentionGroups = { priority_words: "attention-priority", due_words: "attention-due", pay_words: "attention-pay" };
function fillAttentionCriteria(value) {
  for (const [group, id] of Object.entries(attentionGroups)) el(id).value = value[group].join("\n");
}
async function loadAttentionCriteria() {
  const ticket = ++attentionForm.ticket;
  const account = Number(el("attention-account").value) || null;
  attentionForm.account = null;
  el("attention-fields").hidden = true;
  el("attention-status").textContent = "";
  if (!account) return;
  try {
    const data = await invoke("attention_settings", {account});
    if (ticket !== attentionForm.ticket) return;
    attentionForm.account = account;
    attentionForm.defaults = data.defaults;
    fillAttentionCriteria(data.settings);
    el("attention-fields").hidden = false;
  } catch (err) {
    if (ticket === attentionForm.ticket) el("attention-status").textContent = String(err);
  }
}
SETTINGS_PAGES.general.addEventListener("show", () => {
  const select = el("attention-account");
  select.replaceChildren(...state.accounts.map(account => new Option(account.email, account.id)));
  if (state.account !== null) select.value = String(state.account);
  loadAttentionCriteria();
});
el("attention-account").onchange = loadAttentionCriteria;
el("attention-defaults").onclick = () => {
  if (attentionForm.defaults) fillAttentionCriteria(attentionForm.defaults);
  el("attention-status").textContent = t("Original suggestions loaded. Save to apply them.");
};
async function saveAttentionCriteria(account, settings) {
  const saved = await invoke("set_attention_settings", {account, settings});
  // Re-evaluate local rules now, without triggering paid model calls.
  await invoke("find_urgent", {account, useModel:false, onProgress:new window.__TAURI__.core.Channel()});
  if (state.account === account) await reload({keepPosition:true});
  return saved;
}
el("attention-save").onclick = async () => {
  const account = attentionForm.account;
  if (!account) return;
  const button = el("attention-save");
  button.disabled = true;
  el("attention-account").disabled = true;
  try {
    const settings = Object.fromEntries(Object.entries(attentionGroups).map(([group,id]) => [group,el(id).value.split("\n").map(s=>s.trim()).filter(Boolean)]));
    const saved = await saveAttentionCriteria(account, settings);
    if (attentionForm.account === account) {
      fillAttentionCriteria(saved);
      el("attention-status").textContent = t("Attention criteria saved for this account.");
    }
  } catch (err) {
    el("attention-status").textContent = String(err);
  } finally {
    button.disabled = false;
    el("attention-account").disabled = false;
  }
};

// Files for the reply: dropped anywhere on the conversation, or picked.
const conversationFiles = fileTray(el("conversation-files"));
acceptDrops(el("conversation"), conversationFiles);
el("conversation-attach").onclick = () => pickFiles(conversationFiles);
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
