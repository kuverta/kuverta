// The assistant, and the tasks it — or you — set up.
//
// A conversation with a model that has tools on this account's mail: it looks
// (search, list, read, urgency, conversations) and acts (move, archive, trash,
// mark, file, draft a reply, create tasks). The core runs the loop; this shows
// the conversation, what the assistant does as it does it, and a card for each
// thing a person may want to take back or finish: an Undo for every change, a
// reply to read and send, a task to check.
//
// The Tasks tab lists the standing tasks and what they are waiting to be told
// yes or no about. Tasks run after every sync.

const assistant = {
  panel: el("assistant"),
  log: el("chat-log"),
  input: el("chat-input"),
  // The conversation so far, as the core wants it back. Per account: a new
  // account is a new conversation.
  turns: [],
  account: null,
  busy: false,
};

const SUGGESTIONS = [
  "What needs my attention today?",
  "Summarise my unread mail from people.",
  "Find my invoices from this month.",
  "From now on, move invoices into a folder called Rechnungen.",
  "Draft replies to the support requests from this week — I'll check them.",
];

/// A channel for a command to report on as it goes. Outside the app — in the
/// browser tests — there is none, and an object with `onmessage` stands in.
function progressChannel(onmessage) {
  const Channel = window.__TAURI__?.core?.Channel;
  const channel = Channel ? new Channel() : {};
  channel.onmessage = onmessage;
  return channel;
}

// -- opening and closing ------------------------------------------------------------

function assistantOpen() {
  return !assistant.panel.hidden;
}

function setAssistantOpen(open) {
  assistant.panel.hidden = !open;
  document.body.classList.toggle("assistant-open", open);
  try {
    localStorage.setItem("assistantOpen", open ? "yes" : "no");
  } catch {
    // Lasts until the window closes.
  }
  // The list's pool is sized to its height, and its width changed.
  buildPool();
  render(true);
  if (open) {
    showAssistantTab(currentTab());
    if (currentTab() === "chat") assistant.input.focus();
  }
}

function currentTab() {
  return assistant.panel.querySelector(".assistant-tabs .active")?.dataset.tab ?? "chat";
}

function showAssistantTab(tab) {
  for (const button of assistant.panel.querySelectorAll(".assistant-tabs [data-tab]")) {
    button.classList.toggle("active", button.dataset.tab === tab);
  }
  el("assistant-chat").hidden = tab !== "chat";
  el("assistant-tasks").hidden = tab !== "tasks";
  if (tab === "chat") ensureConversation();
  if (tab === "tasks") refreshTasks();
}

el("assistant-toggle").onclick = () => setAssistantOpen(!assistantOpen());
el("assistant-close").onclick = () => setAssistantOpen(false);
for (const button of assistant.panel.querySelectorAll(".assistant-tabs [data-tab]")) {
  button.onclick = () => showAssistantTab(button.dataset.tab);
}
el("assistant-new").onclick = () => {
  assistant.turns = [];
  assistant.log.textContent = "";
  ensureConversation();
  showAssistantTab("chat");
};

// -- the conversation ------------------------------------------------------------------

/// A fresh conversation for the account on screen, with suggestions to start.
function ensureConversation() {
  if (assistant.account !== state.account) {
    assistant.account = state.account;
    assistant.turns = [];
    assistant.log.textContent = "";
  }
  if (!assistant.log.childElementCount) {
    const box = document.createElement("div");
    box.className = "chat-suggestions";
    const hint = document.createElement("div");
    hint.className = "hint";
    hint.textContent = state.email
      ? `Ask about the mail on ${state.email}, or say what to do with it. Everything it changes can be undone, and it never sends mail itself.`
      : "Choose a mail account first.";
    box.append(hint);
    for (const text of SUGGESTIONS) {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = text;
      button.onclick = () => {
        assistant.input.value = text;
        sendToAssistant();
      };
      box.append(button);
    }
    assistant.log.append(box);
  }
  showAssistantModel();
}

/// Which model answers, and — for a hosted one — that mail leaves the computer.
async function showAssistantModel(reply = null) {
  const note = el("chat-model");
  let model = reply?.model;
  let local = reply?.local ?? true;
  if (!model) {
    try {
      const tasks = await invoke("ai_tasks");
      const job = tasks.find((t) => t.task === "assistant");
      const providers = await invoke("ai_providers");
      const provider = providers.find((p) => p.id === job?.provider_id);
      model = job?.model;
      local = provider?.local ?? true;
    } catch {
      return;
    }
  }
  if (!model) return;
  note.textContent = local
    ? `${model}, on this computer. A small model may miss things; a larger one (Settings → Models → Assistant) does better.`
    : `${model}, hosted: what the assistant reads is sent to that service.`;
  note.classList.toggle("warn", !local);
}

function appendChat(className, text) {
  assistant.log.querySelector(".chat-suggestions")?.remove();
  const item = document.createElement("div");
  item.className = className;
  item.textContent = text;
  assistant.log.append(item);
  assistant.log.scrollTop = assistant.log.scrollHeight;
  return item;
}

async function sendToAssistant() {
  const text = assistant.input.value.trim();
  if (!text || assistant.busy) return;
  if (state.account === null || state.postbox) {
    say("choose a mail account first", true);
    return;
  }
  ensureConversation();
  assistant.busy = true;
  el("chat-send").disabled = true;
  assistant.input.value = "";
  appendChat("chat-msg user", text);
  const thinking = appendChat("chat-thinking", "Thinking…");

  const channel = progressChannel((event) => {
    showEvent(event);
    assistant.log.append(thinking);
    assistant.log.scrollTop = assistant.log.scrollHeight;
  });
  const account = state.account;
  try {
    const result = await invoke("assistant_ask", {
      account,
      turns: assistant.turns,
      message: text,
      onEvent: channel,
    });
    thinking.remove();
    if (assistant.account === account) assistant.turns = result.turns;
    appendChat("chat-msg assistant", result.reply.trim() || "(no answer)");
    showAssistantModel(result);
    if (result.events.some((e) => e.kind === "changed" || e.kind === "task_created")) {
      await reload({ keepPosition: true });
      await refreshAssistantBadge();
    }
  } catch (err) {
    thinking.remove();
    appendChat("chat-msg error", String(err));
  } finally {
    assistant.busy = false;
    el("chat-send").disabled = false;
    assistant.input.focus();
  }
}

/// What the assistant did, as a line or a card.
function showEvent(event) {
  switch (event.kind) {
    case "looked":
      appendChat("chat-activity", event.what);
      break;
    case "failed":
      appendChat("chat-activity failed", event.what);
      break;
    case "changed":
      assistant.log.append(changeCard(event));
      break;
    case "draft":
      assistant.log.append(draftCard(event));
      break;
    case "task_created":
      assistant.log.append(taskCard(event));
      break;
    case "message":
      assistant.log.append(messageCard(event));
      break;
  }
  assistant.log.scrollTop = assistant.log.scrollHeight;
}

function card(title, sub) {
  const box = document.createElement("div");
  box.className = "chat-card";
  const head = document.createElement("div");
  head.className = "card-title";
  head.textContent = title;
  box.append(head);
  if (sub) {
    const line = document.createElement("div");
    line.className = "card-sub";
    line.textContent = sub;
    box.append(line);
  }
  const actions = document.createElement("div");
  actions.className = "card-actions";
  box.append(actions);
  return { box, actions };
}

function cardButton(actions, label, action, className = "") {
  const button = document.createElement("button");
  button.type = "button";
  button.textContent = label;
  if (className) button.className = className;
  button.onclick = async () => {
    button.disabled = true;
    try {
      await action(button);
    } catch (err) {
      say(String(err), true);
      button.disabled = false;
    }
  };
  actions.append(button);
  return button;
}

function changeCard(event) {
  const { box, actions } = card(event.what, event.changes.length ? "Queued — it reaches the server at the next sync." : "");
  if (event.changes.length) {
    cardButton(actions, "Undo", async (button) => {
      const cancelled = await invoke("cancel_changes", { ids: event.changes });
      box.classList.add("done");
      button.textContent = cancelled ? `Undone (${cancelled})` : "Already sent — undo it in the mailbox";
      await reload({ keepPosition: true });
    });
  }
  return box;
}

function draftCard(event) {
  const { box, actions } = card(`Reply to ${event.to}`, event.subject);
  const body = document.createElement("textarea");
  body.value = event.body;
  box.insertBefore(body, actions);
  cardButton(actions, "Send", async (button) => {
    if (!confirm(`Send this reply to ${event.to}?`)) {
      button.disabled = false;
      return;
    }
    await sendReply(event.message_id, body.value);
    box.classList.add("done");
    button.textContent = "Sent";
  }, "primary");
  cardButton(actions, "Open in compose", async () => {
    await openReplyInCompose(event.message_id, body.value);
  });
  return box;
}

/// A message the assistant found: open it, or go straight to an attachment —
/// the ticket, the QR code, the contract is often there rather than in the text.
function messageCard(event) {
  const { box, actions } = card(
    event.subject,
    [event.from, formatDate(event.date_utc)].filter(Boolean).join("  ·  "),
  );
  box.classList.add("found");
  if (event.note) {
    const note = document.createElement("div");
    note.className = "card-note";
    note.textContent = event.note;
    box.insertBefore(note, actions);
  }
  const own = event.attachments.filter((a) => !a.inline);
  if (own.length) {
    const files = document.createElement("div");
    files.className = "attachments";
    const account = assistant.account;
    for (const attachment of own) files.append(attachmentChip(account, event.id, attachment));
    box.insertBefore(files, actions);
  }
  cardButton(actions, "Open message", async (button) => {
    button.disabled = false;
    await openWholeMessage(event.id);
  }, "primary");
  return box;
}

function taskCard(event) {
  const { box, actions } = card(
    `New task: ${event.name}`,
    `For mail where ${event.rules}: ${event.what}. ${event.matching} message${event.matching === 1 ? "" : "s"} match now.`,
  );
  cardButton(actions, "Check it in Tasks", async (button) => {
    button.disabled = false;
    showAssistantTab("tasks");
  });
  cardButton(actions, "Delete it", async (button) => {
    await invoke("delete_task", { id: event.id });
    box.classList.add("done");
    button.textContent = "Deleted";
  }, "danger");
  return box;
}

/// Sends a reply the person has read: threaded as a reply to the message,
/// with its quote, as any reply is.
async function sendReply(messageId, body) {
  const sent = await invoke("send", {
    email: state.email,
    draft: {
      to: [],
      cc: [],
      bcc: [],
      subject: "",
      body: `${body.trim()}\n`,
      reply_to: messageId,
      reply_all: false,
      forward: null,
      sign: false,
      encrypt: false,
    },
  });
  say(sent.filing_error ? `sent — but not filed: ${sent.filing_error}` : `sent to ${sent.recipients.join(", ")}`);
}

/// Opens compose on a reply with the drafted text in it, for changing first.
async function openReplyInCompose(messageId, body) {
  closeCompose();
  compose.replyTo = messageId;
  compose.what.textContent = "Reply";
  try {
    const preview = await invoke("preview", {
      email: state.email,
      draft: { to: [], cc: [], bcc: [], subject: "", body: "", reply_to: messageId, reply_all: false, forward: null },
    });
    compose.subject.value = preview.subject;
    compose.to.value = preview.recipients.join(", ");
  } catch {
    // The core fills in who and what when it sends; the fields are for show.
  }
  compose.body.value = `${body.trim()}\n`;
  compose.pane.hidden = false;
  reading.hidden = true;
  emptyPane.hidden = true;
  compose.body.focus();
  await refreshEnvelope();
}

el("chat-send").onclick = sendToAssistant;
assistant.input.addEventListener("keydown", (event) => {
  if (event.key === "Enter" && !event.shiftKey) {
    event.preventDefault();
    sendToAssistant();
  }
  if (event.key === "Escape") assistant.input.blur();
});

// -- tasks -------------------------------------------------------------------------

async function refreshAssistantBadge() {
  if (state.account === null || state.postbox) return;
  let pending = 0;
  try {
    pending = (await invoke("proposals", { account: state.account })).length;
  } catch {
    pending = 0;
  }
  for (const id of ["assistant-badge", "tasks-badge"]) {
    el(id).textContent = String(pending);
    el(id).hidden = pending === 0;
  }
  el("assistant-badge").title = `${pending} waiting for your approval`;
}

async function refreshTasks() {
  if (state.account === null) return;
  const account = state.account;
  let tasks = [];
  let proposals = [];
  try {
    [tasks, proposals] = await Promise.all([
      invoke("tasks", { account }),
      invoke("proposals", { account }),
    ]);
  } catch (err) {
    el("tasks-status").textContent = String(err);
    return;
  }
  if (state.account !== account) return;

  const list = el("task-list");
  list.textContent = "";
  for (const task of tasks) list.append(taskItem(task));

  const waiting = el("proposal-list");
  waiting.textContent = "";
  for (const proposal of proposals) waiting.append(proposalItem(proposal));
  el("proposals-approve-all").hidden = !proposals.some((p) => p.kind !== "reply");
  await refreshAssistantBadge();
}

function taskItem(task) {
  const item = document.createElement("div");
  item.className = "item";
  const on = document.createElement("input");
  on.type = "checkbox";
  on.checked = task.enabled;
  on.title = task.enabled ? "On — runs after every sync" : "Off";
  on.onchange = async () => {
    await invoke("set_task_enabled", { id: task.id, enabled: on.checked }).catch((err) => say(String(err), true));
    await refreshTasks();
  };
  const text = document.createElement("div");
  text.className = "grow";
  const title = document.createElement("div");
  title.className = "title";
  title.textContent = task.name;
  const what = document.createElement("div");
  what.className = "sub";
  what.textContent = `When ${describeQuery(task.query)}: ${task.action_text}${task.review ? " — asks first" : ""}`;
  text.append(title, what);
  if (task.last_summary) {
    const last = document.createElement("div");
    last.className = "sub";
    last.textContent = `Last run: ${task.last_summary}`;
    text.append(last);
  }
  const edit = document.createElement("button");
  edit.type = "button";
  edit.textContent = "Edit";
  edit.onclick = () => openTaskEditor(task);
  item.append(on, text);
  if (task.pending) {
    const chip = document.createElement("span");
    chip.className = "chip accent";
    chip.textContent = `${task.pending} waiting`;
    item.append(chip);
  }
  item.append(edit);
  return item;
}

const PROPOSAL_WORDS = {
  move: (p) => `Move to ${p.target}`,
  archive: () => "Archive",
  trash: () => "Move to Trash",
  mark_read: () => "Mark read",
  file: (p) => `File as ${p.target}`,
  reply: () => "Send this reply",
};

function proposalItem(proposal) {
  const item = document.createElement("div");
  item.className = "item";
  const text = document.createElement("div");
  text.className = "grow";
  const title = document.createElement("div");
  title.className = "title";
  title.textContent = (PROPOSAL_WORDS[proposal.kind] ?? (() => proposal.kind))(proposal);
  const about = document.createElement("div");
  about.className = "sub";
  about.textContent = [proposal.subject ? `“${proposal.subject}”` : "", proposal.from ? `from ${proposal.from}` : "", proposal.task_name ? `· ${proposal.task_name}` : ""]
    .filter(Boolean)
    .join(" ");
  text.append(title, about);
  if (proposal.reason) {
    const why = document.createElement("div");
    why.className = "sub";
    why.textContent = proposal.reason;
    text.append(why);
  }
  let body = null;
  if (proposal.kind === "reply") {
    body = document.createElement("textarea");
    body.className = "reply-preview";
    body.rows = 5;
    body.value = proposal.body ?? "";
    text.append(body);
  }
  item.append(text);

  const button = (label, action, className = "") => {
    const b = document.createElement("button");
    b.type = "button";
    b.textContent = label;
    if (className) b.className = className;
    b.onclick = async () => {
      b.disabled = true;
      try {
        await action();
        await refreshTasks();
        await reload({ keepPosition: true });
      } catch (err) {
        say(String(err), true);
        b.disabled = false;
      }
    };
    item.append(b);
  };
  if (proposal.kind === "reply") {
    button("Send", async () => {
      if (!confirm(`Send this reply${proposal.from ? ` to ${proposal.from}` : ""}?`)) throw new Error("not sent");
      await sendReply(proposal.message_id, body.value);
      await invoke("settle_proposal", { id: proposal.id, state: "done", detail: "sent" });
    }, "primary");
    button("Edit in compose", async () => {
      await openReplyInCompose(proposal.message_id, body.value);
      await invoke("settle_proposal", { id: proposal.id, state: "done", detail: "opened in compose" });
    });
  } else {
    button("Approve", async () => {
      await invoke("approve_proposal", { account: state.account, id: proposal.id });
    }, "primary");
  }
  button("Reject", async () => {
    await invoke("settle_proposal", { id: proposal.id, state: "rejected", detail: null });
  });
  return item;
}

el("proposals-approve-all").onclick = async () => {
  const proposals = await invoke("proposals", { account: state.account });
  let done = 0;
  for (const proposal of proposals.filter((p) => p.kind !== "reply")) {
    try {
      await invoke("approve_proposal", { account: state.account, id: proposal.id });
      done += 1;
    } catch (err) {
      say(String(err), true);
    }
  }
  say(`approved ${done} — each can be undone from the list with z`);
  await refreshTasks();
  await reload({ keepPosition: true });
};

/// Runs the account's tasks. Quiet unless something happened or `loud`.
async function runTasks({ loud = false } = {}) {
  if (state.account === null || state.postbox) return;
  const status = el("tasks-status");
  const channel = progressChannel(([done, total]) => {
    status.textContent = total ? `asking the model: ${done} of ${total}` : "";
  });
  status.textContent = "running…";
  try {
    const runs = await invoke("run_tasks", {
      account: state.account,
      useModel: askModelForUrgency(),
      onProgress: channel,
    });
    const done = runs.reduce((n, r) => n + r.done, 0);
    const proposed = runs.reduce((n, r) => n + r.proposed, 0);
    status.textContent = runs.length ? `${done} done, ${proposed} waiting for you` : "";
    if (done || proposed || loud) {
      say(runs.length ? `tasks: ${done} done, ${proposed} waiting for you` : "there are no tasks yet");
      await reload({ keepPosition: true });
    }
  } catch (err) {
    status.textContent = String(err);
    if (loud) say(String(err), true);
  }
  await refreshAssistantBadge();
  if (assistantOpen() && currentTab() === "tasks") await refreshTasks();
}

el("tasks-run").onclick = () => runTasks({ loud: true });

// -- the task editor -----------------------------------------------------------------

const taskSheet = el("task-sheet");
const taskForm = el("task-form");
const taskRules = el("task-rules");
const taskEditing = { id: null };

function openTaskEditor(task = null) {
  if (state.account === null) return;
  taskEditing.id = task?.id ?? null;
  el("task-form-title").textContent = task ? `Edit “${task.name}”` : "New task";
  el("task-delete").hidden = !task;
  taskForm.name.value = task?.name ?? "";
  taskForm.match_all.value = task?.query?.match_all === false ? "any" : "all";
  taskRules.textContent = "";
  for (const rule of task?.query?.rules?.length ? task.query.rules : [{ field: "subject", op: "contains", value: "" }]) {
    taskRules.append(ruleRow(rule));
  }
  const folders = taskForm.folder;
  folders.textContent = "";
  for (const folder of state.folders) folders.append(new Option(folder.name, folder.name));
  const action = task?.action ?? { kind: "move" };
  taskForm.action.value = action.kind;
  if (action.folder) folders.value = action.folder;
  if (action.category) taskForm.category.value = action.category;
  taskForm.instruction.value = action.instruction ?? "";
  taskForm.review.checked = task?.review ?? false;
  taskForm.include_existing.checked = false;
  taskForm.include_existing.closest("label").hidden = Boolean(task);
  syncTaskAction();
  taskSheet.hidden = false;
  taskForm.name.focus();
  taskPreviewSoon();
}

function syncTaskAction() {
  const kind = taskForm.action.value;
  for (const label of taskForm.querySelectorAll("[data-for]")) {
    label.hidden = !label.dataset.for.split(" ").includes(kind);
  }
  const reply = kind === "reply";
  taskForm.review.disabled = reply;
  if (reply) taskForm.review.checked = true;
  el("task-note").textContent =
    reply
      ? "Replies always wait for you: nothing a model writes is sent without you reading it."
      : kind === "decide"
        ? "The model reads each message and picks move, archive, trash, mark read, file or a reply — or leaves it."
        : "";
}

taskForm.action.addEventListener("change", syncTaskAction);
el("task-rule-add").onclick = () => taskRules.append(ruleRow({ field: "from", op: "contains", value: "" }));

let taskPreviewTimer = null;
function taskPreviewSoon() {
  clearTimeout(taskPreviewTimer);
  taskPreviewTimer = setTimeout(async () => {
    const query = taskQuery();
    query.rules = query.rules.filter((r) => r.value !== "");
    const out = el("task-preview");
    if (!query.rules.length) {
      out.textContent = "";
      return;
    }
    try {
      const preview = await invoke("preview_smart", { account: state.account, query });
      out.textContent = `${preview.total} message${preview.total === 1 ? "" : "s"} match these rules now.`;
    } catch (err) {
      out.textContent = String(err);
    }
  }, 250);
}
taskRules.addEventListener("input", taskPreviewSoon);
taskRules.addEventListener("change", taskPreviewSoon);

function taskQuery() {
  return {
    match_all: taskForm.match_all.value === "all",
    rules: [...taskRules.querySelectorAll(".rule")].map((row) => ({
      field: row.querySelector("[name=field]").value,
      op: row.querySelector("[name=op]").value,
      value: row.querySelector("[name=value]").value.trim(),
    })),
  };
}

function taskAction() {
  const kind = taskForm.action.value;
  if (kind === "move") return { kind, folder: taskForm.folder.value };
  if (kind === "file") return { kind, category: taskForm.category.value };
  if (kind === "reply" || kind === "decide") return { kind, instruction: taskForm.instruction.value.trim() };
  return { kind };
}

taskForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  try {
    await invoke("save_task", {
      input: {
        id: taskEditing.id,
        account_id: state.account,
        name: taskForm.name.value.trim(),
        query: taskQuery(),
        action: taskAction(),
        review: taskForm.review.checked,
        enabled: true,
        include_existing: taskForm.include_existing.checked,
      },
    });
    closeDialog(taskSheet);
    say("task saved — it runs after every sync");
    await refreshTasks();
  } catch (err) {
    say(String(err), true);
  }
});

el("task-cancel").onclick = () => closeDialog(taskSheet);
el("task-delete").onclick = async () => {
  if (!confirm("Delete this task? What it did stays done.")) return;
  await invoke("delete_task", { id: taskEditing.id });
  closeDialog(taskSheet);
  await refreshTasks();
};
el("task-new").onclick = () => openTaskEditor();

// Opened last time, open again.
try {
  if (localStorage.getItem("assistantOpen") === "yes") {
    document.addEventListener("DOMContentLoaded", () => setTimeout(() => setAssistantOpen(true), 0));
  }
} catch {
  // A preference only.
}
