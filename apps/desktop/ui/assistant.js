// The assistant, and the tasks it — or you — set up.
//
// A conversation with a model that has tools on this account's mail: it looks
// (search, list, read, urgency, conversations) and acts (move, archive, trash,
// mark, file, draft a reply, create tasks). The core runs the loop; this shows
// the conversation, what the assistant does as it does it, and a card for each
// thing a person may want to take back or finish: an Undo for every change, a
// reply to read and send, a task to check.
//
// A document it makes — a letter it wrote, a PDF it signed — is a card too:
// to show, save, attach to a message, or delete. A question that fails —
// the model's service is down, or out of credit — leaves a Retry. Every
// conversation is kept as it goes, and Earlier chats opens one again.
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
  modelChanging: false,
  // The messages this question is about: id and a line to show, in the order
  // they were picked. Emptied once the question has been asked — from then on
  // they are part of the conversation.
  about: [],
  // The conversation as kept: its id in the store once it has been saved,
  // and each exchange as the log showed it, so it can be shown again.
  chatId: null,
  exchanges: [],
};

/// What to do with the messages you handed it, offered as a starting point.
const ABOUT_SUGGESTIONS = [
  t("Reply and confirm."),
  t("What does this need from me?"),
  t("Summarise this for me."),
  t("Write a summary of this I can send to someone."),
];

const SUGGESTIONS = [
  t("What needs my attention today?"),
  t("Summarise my unread mail from people."),
  t("Find my invoices from this month."),
  t("Draft replies to the support requests from this week — I'll check them."),
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
  try {
    localStorage.setItem("assistantOpen", open ? "yes" : "no");
  } catch {
    // Lasts until the window closes.
  }
  // It shares the right-hand column with the sender card, which decides
  // whether the column is there at all and redraws the list if it moved.
  syncAside();
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
  startChat();
  showAssistantTab("chat");
};

/// A fresh, unsaved conversation on the account on screen.
function startChat() {
  assistant.turns = [];
  assistant.chatId = null;
  assistant.exchanges = [];
  assistant.log.textContent = "";
  hideHistory();
  ensureConversation();
}

// -- the messages a question is about --------------------------------------------------

/// Hands messages to the assistant: opens the panel with them attached, ready
/// for "answer this and confirm the appointment".
async function askAboutMessages(rows) {
  if (state.account === null || state.postbox) {
    say(t("the assistant works on mail accounts"), true);
    return;
  }
  if (!rows.length) return;
  ensureConversation();
  assistant.about = rows.map((row) => ({
    id: row.id,
    label: row.subject || t("(no subject)"),
    from: row.from ?? "",
  }));
  if (!assistantOpen()) setAssistantOpen(true);
  showAssistantTab("chat");
  showAbout();
  assistant.input.focus();
}

/// The attached messages, each with a way to take it off again.
function showAbout() {
  const bar = el("chat-about");
  bar.textContent = "";
  if (!assistant.about.length) {
    bar.hidden = true;
    return;
  }
  const head = document.createElement("span");
  head.className = "hint";
  head.textContent = assistant.about.length === 1 ? t("About:") : t("About {count} messages:", { count: assistant.about.length });
  bar.append(head);
  for (const message of assistant.about) {
    const chip = document.createElement("span");
    chip.className = "about-chip";
    const what = document.createElement("span");
    what.className = "name";
    // textContent: a subject is whatever the sender wrote.
    what.textContent = message.label;
    what.title = `${message.label} — ${message.from}`;
    const off = document.createElement("button");
    off.type = "button";
    off.className = "remove";
    off.textContent = "×";
    off.title = t("Leave this one out");
    off.setAttribute("aria-label", t("Leave out {subject}", { subject: message.label }));
    off.onclick = () => {
      assistant.about = assistant.about.filter((m) => m.id !== message.id);
      showAbout();
    };
    chip.append(what, off);
    bar.append(chip);
  }
  bar.hidden = false;
  showSuggestions(ABOUT_SUGGESTIONS);
}

// -- the conversation ------------------------------------------------------------------

/// A fresh conversation for the account on screen, with suggestions to start.
function ensureConversation(suggestions = SUGGESTIONS) {
  if (assistant.account !== state.account) {
    assistant.account = state.account;
    assistant.turns = [];
    assistant.chatId = null;
    assistant.exchanges = [];
    assistant.log.textContent = "";
    hideHistory();
  }
  showSuggestionsInto(suggestions);
}

/// Offers ways to start, until something has been said.
function showSuggestionsInto(suggestions) {
  if (!assistant.log.childElementCount || assistant.log.querySelector(".chat-suggestions")) {
    assistant.log.querySelector(".chat-suggestions")?.remove();
    const box = document.createElement("div");
    box.className = "chat-suggestions";
    const title = document.createElement("h3");
    title.textContent = t("Your mail at a glance");
    const account = document.createElement("p");
    account.className = "chat-account";
    account.textContent = state.email || t("Choose a mail account first.");
    box.append(title, account);
    const add = (parent, label, prompt) => {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = label;
      button.disabled = !state.email;
      button.onclick = () => {
        assistant.input.value = prompt;
        assistant.input.focus();
      };
      parent.append(button);
    };
    if (suggestions === SUGGESTIONS) {
      const actions = document.createElement("div");
      actions.className = "chat-start-actions";
      add(actions, t("Find what matters"), t("Show important messages and open requests with reasons. Do not change anything yet."));
      add(actions, t("Prepare replies"), t("Draft replies to the support requests from this week — I'll check them."));
      add(actions, t("Tidy up"), t("Suggest messages to archive or review as spam. Show your reasons; do not move or delete anything yet."));
      const more = document.createElement("details");
      more.className = "chat-more";
      const summary = document.createElement("summary");
      summary.textContent = t("More suggestions");
      more.append(summary);
      for (const prompt of suggestions) add(more, prompt, prompt);
      box.append(actions, more);
    } else {
      for (const prompt of suggestions) add(box, prompt, prompt);
    }
    assistant.log.append(box);
  }
  showAssistantModel();
}

/// Replaces the offered starting points, when the question has a subject.
function showSuggestions(suggestions) {
  if (assistant.log.querySelector(".chat-suggestions")) showSuggestionsInto(suggestions);
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
  const details = document.createElement("details");
  const summary = document.createElement("summary");
  const label = model.startsWith("claude-") ? "Claude" : model;
  summary.textContent = `${label} · ${local ? t("Local") : t("Online")}`;
  const explanation = document.createElement("p");
  explanation.textContent = local
    ? t("{model} runs on this computer. Mail processed here stays local.", { model })
    : t("{model}, hosted: what the assistant reads is sent to that service.", { model });
  details.append(summary, explanation);
  let loaded = false;
  details.addEventListener("toggle", () => {
    if (details.open && !loaded) {
      loaded = true;
      assistantModelPicker(details);
    }
  });
  note.replaceChildren(details);
  note.classList.toggle("warn", !local);
}

/// Changes only the assistant's saved choice. Loading a list never changes it.
async function assistantModelPicker(details) {
  const box = document.createElement("fieldset");
  box.className = "chat-model-picker";
  box.disabled = assistant.busy || assistant.modelChanging;
  const status = document.createElement("p");
  status.setAttribute("role", "status");
  status.textContent = t("Loading…");
  details.append(box, status);
  try {
    const [providers, tasks] = await Promise.all([invoke("ai_providers"), invoke("ai_tasks")]);
    if (!details.isConnected) return;
    const current = tasks.find((job) => job.task === "assistant");
    const provider = document.createElement("select");
    provider.id = "chat-provider";
    const choice = document.createElement("select");
    choice.id = "chat-model-choice";
    const manual = document.createElement("input");
    manual.id = "chat-model-name";
    manual.setAttribute("aria-label", t("Model name"));
    manual.placeholder = t("Model name");
    manual.hidden = true;
    const save = document.createElement("button");
    save.id = "chat-model-save";
    save.type = "button";
    save.textContent = t("Use model");
    save.disabled = true;
    const privacy = document.createElement("p");
    const label = (text, field) => {
      const node = document.createElement("label");
      node.textContent = t(text);
      node.append(field);
      return node;
    };
    box.append(label("Provider", provider), label("Model", choice), manual, privacy, save);
    for (const item of providers) provider.append(new Option(`${item.label} · ${item.local ? t("Local") : t("Online")}`, String(item.id)));
    provider.value = String(current?.provider_id ?? providers[0]?.id);
    let request = 0;
    const selectedModel = () => choice.value || manual.value.trim();
    const load = async (keep = "") => {
      const ticket = ++request;
      const id = Number(provider.value);
      choice.replaceChildren();
      choice.disabled = true;
      manual.value = "";
      manual.hidden = true;
      save.disabled = true;
      status.textContent = t("Loading…");
      const local = providers.find((item) => item.id === id)?.local === true;
      privacy.textContent = local ? t("Mail processed here stays on this computer.") : t("Chat history and mail the assistant reads are sent to this service.");
      privacy.classList.toggle("warn", !local);
      try {
        const offered = await offeredBy(id, { fresh: true });
        if (ticket !== request || !details.isConnected) return;
        for (const model of offered) {
          if (!model.embedding) choice.append(new Option(model.name, model.name));
        }
        status.textContent = t("Applies to the assistant from the next message.");
      } catch (err) {
        if (ticket !== request || !details.isConnected) return;
        status.textContent = String(err);
      }
      choice.append(new Option(t("Enter model name…"), ""));
      choice.value = keep && [...choice.options].some((option) => option.value === keep) ? keep : "";
      manual.value = keep;
      manual.hidden = Boolean(choice.value);
      choice.disabled = false;
      save.disabled = !selectedModel();
    };
    provider.onchange = () => load();
    choice.onchange = () => {
      manual.hidden = Boolean(choice.value);
      save.disabled = !selectedModel();
    };
    manual.oninput = () => { save.disabled = !selectedModel(); };
    save.onclick = async () => {
      if (assistant.busy || assistant.modelChanging || !selectedModel()) return;
      assistant.modelChanging = true;
      box.disabled = true;
      el("chat-send").disabled = true;
      try {
        await invoke("set_ai_task", { task: "assistant", providerId: Number(provider.value), model: selectedModel() });
        await showAssistantModel();
      } catch (err) {
        status.textContent = String(err);
      } finally {
        assistant.modelChanging = false;
        box.disabled = assistant.busy;
        el("chat-send").disabled = assistant.busy;
      }
    };
    await load(current?.model ?? "");
  } catch (err) {
    status.textContent = String(err);
  }
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

/// The model's answer, with the Markdown it writes in — bold, italics, code,
/// headings and lists — shown as such. Built from text nodes, never as HTML:
/// the answer quotes mail, and mail is written by strangers.
function markdown(text) {
  const out = document.createDocumentFragment();
  let list = null;
  for (const line of text.split("\n")) {
    const item = line.match(/^(\s*)([-*•]|\d+[.)])\s+(.*)$/);
    if (item) {
      const ordered = /\d/.test(item[2]);
      const nested = item[1].length >= 2 && list;
      const tag = ordered ? "OL" : "UL";
      let target = list;
      if (nested) {
        const parent = list.lastElementChild;
        target = parent.querySelector(":scope > ul, :scope > ol");
        if (!target) {
          target = document.createElement(tag.toLowerCase());
          parent.append(target);
        }
      } else if (!list || list.tagName !== tag) {
        list = document.createElement(tag.toLowerCase());
        if (ordered) list.start = Number.parseInt(item[2], 10) || 1;
        out.append(list);
        target = list;
      }
      const li = document.createElement("li");
      li.append(...inline(item[3]));
      target.append(li);
      continue;
    }
    if (!line.trim()) {
      list = null;
      continue;
    }
    list = null;
    const heading = line.match(/^#{1,6}\s+(.*)$/);
    const block = document.createElement(heading ? "h4" : "p");
    block.append(...inline(heading ? heading[1] : line));
    out.append(block);
  }
  return out;
}

/// **bold**, *italic* or _italic_, and `code`, as nodes.
function inline(text) {
  const nodes = [];
  const pattern = /\*\*(.+?)\*\*|`([^`]+)`|\*([^*\s][^*]*?)\*|(?<![\w])_([^_]+)_(?![\w])/g;
  let at = 0;
  for (const match of text.matchAll(pattern)) {
    if (match.index > at) nodes.push(document.createTextNode(text.slice(at, match.index)));
    const [, bold, code, italic, underscored] = match;
    const node = document.createElement(bold ? "strong" : code ? "code" : "em");
    node.append(...(bold ? inline(bold) : [document.createTextNode(code ?? italic ?? underscored)]));
    nodes.push(node);
    at = match.index + match[0].length;
  }
  if (at < text.length) nodes.push(document.createTextNode(text.slice(at)));
  return nodes;
}

/// Asks what is in the box — or, on a Retry, the question that failed, with
/// the messages it was about.
async function sendToAssistant(again = null) {
  const text = again ? again.text : assistant.input.value.trim();
  if (!text || assistant.busy || assistant.modelChanging) return;
  if (state.account === null || state.postbox) {
    say(t("choose a mail account first"), true);
    return;
  }
  ensureConversation();
  hideHistory();
  assistant.busy = true;
  el("chat-send").disabled = true;
  assistant.panel.querySelectorAll(".chat-model-picker").forEach((box) => { box.disabled = true; });
  if (!again) assistant.input.value = "";
  // Asked with the messages that were attached; from now on they are part of
  // the conversation, so the bar empties.
  const about = again ? again.about : assistant.about;
  if (!again) {
    assistant.about = [];
    showAbout();
  }
  if (again) {
    again.error.remove();
  } else {
    appendChat("chat-msg user", text);
    if (about.length) {
      appendChat("chat-activity", t("about {messages}", { messages: about.map((m) => `“${m.label}”`).join(", ") }));
    }
  }
  const thinking = appendChat("chat-thinking", t("Thinking…"));

  const channel = progressChannel((event) => {
    showEvent(event);
    assistant.log.append(thinking);
    assistant.log.scrollTop = assistant.log.scrollHeight;
  });
  const account = state.account;
  let invoiceProposal=null;
  try {
    const result = await invoke("assistant_ask", {
      account,
      turns: assistant.turns,
      message: text,
      about: about.map((m) => m.id),
      onEvent: channel,
    });
    thinking.remove();
    const reply = result.reply.trim();
    appendChat("chat-msg assistant", "").replaceChildren(markdown(reply || t("(no answer)")));
    assistant.log.scrollTop = assistant.log.scrollHeight;
    showAssistantModel(result);
    if (assistant.account === account) {
      assistant.turns = result.turns;
      assistant.exchanges.push({
        question: text,
        about: about.map((m) => ({ id: m.id, label: m.label, from: m.from })),
        events: result.events,
        reply,
        model: result.model,
        local: result.local,
      });
      await keepChat(account);
    }
    if (result.events.some((e) => e.kind === "changed" || e.kind === "task_created")) {
      await reload({ keepPosition: true });
      await refreshAssistantBadge();
    }
    const collections=result.events.filter(e=>e.kind==="invoice_collection_suggested");
    // A live answer can open one preview. Replaying history only renders cards.
    if(collections.length===1 && assistant.account===account && state.account===account)invoiceProposal=collections[0].proposal;
  } catch (err) {
    thinking.remove();
    // The question stays asked and the conversation as it was, so Retry
    // sends exactly it again — after topping up a hosted service's credit,
    // or once the model is back.
    const error = appendChat("chat-msg error", String(err));
    const retry = document.createElement("button");
    retry.type = "button";
    retry.className = "retry";
    retry.textContent = t("Retry");
    retry.onclick = () => sendToAssistant({ text, about, error });
    error.append(retry);
  } finally {
    assistant.busy = false;
    el("chat-send").disabled = false;
    assistant.panel.querySelectorAll(".chat-model-picker").forEach((box) => { box.disabled = false; });
    assistant.input.focus();
  }
  if(invoiceProposal && ![...document.querySelectorAll(".sheet")].some(sheet=>!sheet.hidden)) {
    try { await openAssistantInvoiceCollection(invoiceProposal); }
    catch(error) { say(String(error),true); }
  }
}

// -- keeping the conversation ---------------------------------------------------------

/// Keeps the conversation as it now stands. A new one gets its id here; from
/// then on it is the same chat, added to. Quietly: a chat that could not be
/// kept is still on screen.
async function keepChat(account) {
  try {
    const kept = await invoke("save_chat", {
      id: assistant.chatId,
      account,
      turns: assistant.turns,
      log: assistant.exchanges,
    });
    if (assistant.account === account) assistant.chatId = kept.id;
  } catch (err) {
    console.warn("the chat could not be kept", err);
  }
}

/// Shows a kept conversation again, as it was shown the first time, and
/// goes on from it.
async function openChat(id) {
  let chat;
  try {
    chat = await invoke("chat", { id });
  } catch (err) {
    say(String(err), true);
    return;
  }
  if (chat.account_id !== state.account) return;
  assistant.account = state.account;
  assistant.turns = chat.turns;
  assistant.chatId = chat.id;
  assistant.exchanges = Array.isArray(chat.log) ? chat.log : [];
  assistant.about = [];
  showAbout();
  hideHistory();
  assistant.log.textContent = "";
  for (const exchange of assistant.exchanges) {
    appendChat("chat-msg user", exchange.question);
    if (exchange.about?.length) {
      appendChat("chat-activity", t("about {messages}", { messages: exchange.about.map((m) => `“${m.label}”`).join(", ") }));
    }
    for (const event of exchange.events ?? []) showEvent(event);
    appendChat("chat-msg assistant", "").replaceChildren(markdown(exchange.reply || t("(no answer)")));
  }
  assistant.log.scrollTop = assistant.log.scrollHeight;
  const last = assistant.exchanges[assistant.exchanges.length - 1];
  showAssistantModel(last?.model ? { model: last.model, local: last.local } : null);
  assistant.input.focus();
}

function hideHistory() {
  el("chat-history").hidden = true;
  assistant.log.hidden = false;
}

/// The earlier conversations on this account, newest first, in place of the
/// log until one is picked.
async function showHistory() {
  if (state.account === null || state.postbox) return;
  const panel = el("chat-history");
  if (!panel.hidden) {
    hideHistory();
    return;
  }
  let chats = [];
  try {
    chats = await invoke("chats", { account: state.account });
  } catch (err) {
    say(String(err), true);
    return;
  }
  panel.textContent = "";
  const head = document.createElement("div");
  head.className = "chat-history-head";
  const title = document.createElement("span");
  title.textContent = t("Earlier chats");
  const back = document.createElement("button");
  back.type = "button";
  back.className = "tool";
  back.textContent = t("Back");
  back.onclick = hideHistory;
  head.append(title, back);
  panel.append(head);
  if (!chats.length) {
    const none = document.createElement("p");
    none.className = "hint";
    none.textContent = t("Nothing yet. Every conversation is kept as it goes, and shows here.");
    panel.append(none);
  }
  for (const chat of chats) {
    const row = document.createElement("div");
    row.className = `item chat-history-item${chat.id === assistant.chatId ? " current" : ""}`;
    const open = document.createElement("button");
    open.type = "button";
    open.className = "grow open";
    const name = document.createElement("div");
    name.className = "title";
    // textContent: the title is the person's own question.
    name.textContent = chat.title;
    const when = document.createElement("div");
    when.className = "sub";
    when.textContent = formatDate(chat.updated_at);
    open.append(name, when);
    open.onclick = () => openChat(chat.id);
    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "tool icon-only";
    remove.textContent = "×";
    remove.title = t("Delete this chat");
    remove.setAttribute("aria-label", t("Delete the chat “{title}”", { title: chat.title }));
    remove.onclick = async () => {
      try {
        await invoke("delete_chat", { id: chat.id });
      } catch (err) {
        say(String(err), true);
        return;
      }
      if (chat.id === assistant.chatId) assistant.chatId = null;
      row.remove();
    };
    row.append(open, remove);
    panel.append(row);
  }
  assistant.log.hidden = true;
  panel.hidden = false;
}

el("assistant-history").onclick = showHistory;

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
    case "document":
      assistant.log.append(documentCard(event));
      break;
    case "remembered":
      assistant.log.append(rememberedCard(event));
      break;
    case "filing_suggested":
      assistant.log.append(filingSuggestionCard(event.proposal));
      break;
    case "invoice_collection_suggested":
      assistant.log.append(invoiceCollectionCard(event.proposal));
      break;
    case "attention_suggested":
      assistant.log.append(attentionSuggestionCard(event));
      break;
    case "skill_downloaded":
      assistant.log.append(skillCard(event));
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

function invoiceCollectionCard(proposal) {
  const {box,actions}=card(`${t("Collect invoices")} · ${proposal.profile_name}`,
    t("Invoice dates {from} to {to}. One joint preview for this profile; files are saved only after your approval.",{from:proposal.from,to:proposal.to}));
  box.classList.add("invoice-collection-card");
  cardButton(actions,t("Open joint preview"),async button=>{
    try { await openAssistantInvoiceCollection(proposal); }
    finally { button.disabled=false; }
  });
  return box;
}

/// A note the assistant kept, with the way to take it back.
function rememberedCard(event) {
  const { box, actions } = card(t("Remembered"), event.text);
  cardButton(actions, t("Forget"), async (button) => {
    await invoke("forget_memory", { account: state.account, id: event.id });
    box.classList.add("done");
    button.textContent = t("Forgotten");
  });
  cardButton(actions, t("All notes"), async () => openSettings("memory"));
  return box;
}

function attentionSuggestionCard(event) {
  const email = state.accounts.find(a => a.id === event.account)?.email ?? String(event.account);
  const labels = {priority_words:"Personal topics to review",due_words:"Deadline terms",pay_words:"Payment terms"};
  const {box, actions} = card(t("Suggested attention criteria"), `${email} · ${t(labels[event.group] ?? event.group)}: ${event.words.join(", ")} — ${event.reason}`);
  cardButton(actions, t("Apply suggestion"), async button => {
    if (!Object.hasOwn(labels, event.group)) throw new Error("Unknown keyword group");
    const {settings} = await invoke("attention_settings", {account:event.account});
    settings[event.group] = [...new Set([...settings[event.group], ...event.words])];
    await saveAttentionCriteria(event.account, settings);
    button.textContent = t("Applied");
    box.classList.add("done");
  });
  return box;
}

/// A skill the assistant fetched. It is instructions from somebody else, and
/// it does nothing until the person has read it in the settings and accepted
/// it there — which is where the one button takes them.
function skillCard(event) {
  const { box, actions } = card(
    t("Skill downloaded: {name}", { name: event.name }),
    t("From {source}. It does nothing until you have read and accepted it in the settings.", { source: event.source }),
  );
  cardButton(actions, t("Read and accept…"), async () => {
    research.open = event.id;
    await openSettings("research");
  });
  return box;
}

function changeCard(event) {
  const { box, actions } = card(event.what, event.changes.length ? t("Queued — it reaches the server at the next sync.") : "");
  if (event.changes.length) {
    cardButton(actions, t("Undo"), async (button) => {
      const cancelled = await invoke("cancel_changes", { ids: event.changes });
      box.classList.add("done");
      button.textContent = cancelled
        ? t("Undone ({count})", { count: cancelled })
        : t("Already sent — undo it in the mailbox");
      await reload({ keepPosition: true });
    });
  }
  return box;
}

function draftCard(event) {
  const reply = event.message_id != null;
  const { box, actions } = card(
    reply ? t("Reply to {to}", { to: event.to }) : t("New message to {to}", { to: event.to }),
    event.subject,
  );
  const body = document.createElement("textarea");
  body.value = event.body;
  box.insertBefore(body, actions);
  // Documents the assistant made that go with it: shown as chips, and sent
  // or opened in compose with the message.
  const documents = event.documents ?? [];
  if (documents.length) {
    const files = document.createElement("div");
    files.className = "attachments";
    for (const doc of documents) files.append(documentChip(doc));
    box.insertBefore(files, actions);
  }
  cardButton(actions, t("Send"), async (button) => {
    if (!(await ask(t("Send this to {to}?", { to: event.to }), { yes: t("Send") }))) {
      button.disabled = false;
      return;
    }
    const attachments = await documentAttachments(documents);
    if (reply) {
      await sendReply(event.message_id, body.value, attachments);
    } else {
      await sendNew(event.to, event.subject, body.value, attachments);
    }
    box.classList.add("done");
    button.textContent = t("Sent");
  }, "primary");
  cardButton(actions, t("Open in compose"), async () => {
    const attachments = await documentAttachments(documents);
    if (reply) {
      await openReplyInCompose(event.message_id, body.value);
      compose.files.set(attachments);
      return;
    }
    await openComposeWith({
      to: event.to.split(",").map((address) => address.trim()).filter(Boolean),
      cc: [],
      bcc: [],
      subject: event.subject,
      body: `${body.value.trim()}\n`,
      reply_to: null,
      reply_all: false,
      forward: null,
      attachments,
    });
    compose.what.textContent = t("New message");
  });
  return box;
}

/// A document the assistant made: show it, save it, attach it to a new
/// message, or delete it.
function documentCard(event) {
  const doc = event.document;
  const { box, actions } = card(
    doc.name,
    [
      t("PDF"),
      sizeText(doc.size),
      doc.pages == null ? "" : doc.pages === 1 ? t("one page") : t("{count} pages", { count: doc.pages }),
      doc.signed ? t("signed") : "",
    ]
      .filter(Boolean)
      .join("  ·  "),
  );
  box.classList.add("found");
  if (event.note) {
    const note = document.createElement("div");
    note.className = "card-note";
    note.textContent = event.note;
    box.insertBefore(note, actions);
  }
  cardButton(actions, t("Show"), async (button) => {
    button.disabled = false;
    await openDocument(doc);
  }, "primary");
  // Where the signature landed is the one thing a model cannot check, so
  // the person can always put it where it belongs themselves.
  if (canPlace(doc)) {
    cardButton(actions, t("Move the signature…"), async (button) => {
      button.disabled = false;
      await openPlacer(doc, (moved) => {
        Object.assign(doc, moved);
        box.replaceWith(documentCard({ ...event, document: doc }));
      });
    });
  }
  cardButton(actions, t("Save to Downloads"), async (button) => {
    button.disabled = false;
    const path = await invoke("save_document", { id: doc.id });
    say(t("saved to {path}", { path }));
  });
  cardButton(actions, t("Attach to a new message"), async (button) => {
    button.disabled = false;
    await openComposeWith({
      to: [],
      cc: [],
      bcc: [],
      subject: "",
      body: "",
      reply_to: null,
      reply_all: false,
      forward: null,
      attachments: await documentAttachments([doc]),
    });
    compose.what.textContent = t("New message");
  });
  cardButton(actions, t("Delete it"), async (button) => {
    if (!(await ask(t("Delete {name}?", { name: doc.name }), { yes: t("Delete"), danger: true }))) {
      button.disabled = false;
      return;
    }
    await invoke("delete_document", { id: doc.id });
    box.classList.add("done");
    button.textContent = t("Deleted");
  }, "danger");
  return box;
}

/// Documents as a draft carries them: name, type and bytes as base64.
async function documentAttachments(documents) {
  const out = [];
  for (const doc of documents) {
    const bytes = new Uint8Array(await invoke("document", { id: doc.id }));
    let binary = "";
    for (let at = 0; at < bytes.length; at += 0x8000) {
      binary += String.fromCharCode(...bytes.subarray(at, at + 0x8000));
    }
    out.push({ name: doc.name, content_type: doc.content_type, data: btoa(binary) });
  }
  return out;
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
  cardButton(actions, t("Open message"), async (button) => {
    button.disabled = false;
    await openWholeMessage(event.id);
  }, "primary");
  return box;
}

function taskCard(event) {
  const { box, actions } = card(
    t("New task: {name}", { name: event.name }),
    event.matching === 1
      ? t("For mail where {rules}: {what}. One message matches now.", { rules: event.rules, what: event.what })
      : t("For mail where {rules}: {what}. {count} messages match now.", {
          rules: event.rules,
          what: event.what,
          count: event.matching,
        }),
  );
  cardButton(actions, t("Check it in Tasks"), async (button) => {
    button.disabled = false;
    showAssistantTab("tasks");
  });
  cardButton(actions, t("Delete it"), async (button) => {
    await invoke("delete_task", { id: event.id });
    box.classList.add("done");
    button.textContent = t("Deleted");
  }, "danger");
  return box;
}

/// Sends a new message the person has read: to whoever the assistant named,
/// with the subject it wrote.
async function sendNew(to, subject, body, attachments = []) {
  const sent = await invoke("send", {
    email: state.email,
    draft: {
      to: to.split(",").map((address) => address.trim()).filter(Boolean),
      cc: [],
      bcc: [],
      subject,
      body: `${body.trim()}\n`,
      reply_to: null,
      reply_all: false,
      forward: null,
      sign: false,
      encrypt: false,
      attachments,
    },
  });
  say(
    sent.filing_error
      ? t("sent — but not filed: {error}", { error: sent.filing_error })
      : t("sent to {recipients}", { recipients: sent.recipients.join(", ") }),
  );
}

/// Sends a reply the person has read: threaded as a reply to the message,
/// with its quote, as any reply is.
async function sendReply(messageId, body, attachments = []) {
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
      attachments,
    },
  });
  say(
    sent.filing_error
      ? t("sent — but not filed: {error}", { error: sent.filing_error })
      : t("sent to {recipients}", { recipients: sent.recipients.join(", ") }),
  );
}

/// Opens compose on a reply with the drafted text in it, for changing first.
async function openReplyInCompose(messageId, body) {
  closeCompose();
  compose.replyTo = messageId;
  compose.what.textContent = t("Reply");
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

// A click event is not the saved { text, about, error } used by Retry.
el("chat-send").onclick = () => sendToAssistant();
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
  el("assistant-badge").title = t("{count} waiting for your approval", { count: pending });
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
  on.title = task.enabled ? t("On — runs after every sync") : t("Off");
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
  what.textContent = task.review
    ? t("When {rules}: {what} — asks first", { rules: describeQuery(task.query), what: task.action_text })
    : t("When {rules}: {what}", { rules: describeQuery(task.query), what: task.action_text });
  text.append(title, what);
  if (task.last_summary) {
    const last = document.createElement("div");
    last.className = "sub";
    last.textContent = t("Last run: {summary}", { summary: task.last_summary });
    text.append(last);
  }
  const edit = document.createElement("button");
  edit.type = "button";
  edit.textContent = t("Edit");
  edit.onclick = () => openTaskEditor(task);
  item.append(on, text);
  if (task.pending) {
    const chip = document.createElement("span");
    chip.className = "chip accent";
    chip.textContent = t("{count} waiting", { count: task.pending });
    item.append(chip);
  }
  item.append(edit);
  return item;
}

const PROPOSAL_WORDS = {
  move: (p) => t("Move to {folder}", { folder: p.target }),
  archive: () => t("Archive"),
  trash: () => t("Move to Trash"),
  mark_read: () => t("Mark read"),
  file: (p) => t("File as {category}", { category: p.target }),
  reply: () => t("Send this reply"),
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
  about.textContent = [
    proposal.subject ? `“${proposal.subject}”` : "",
    proposal.from ? t("from {sender}", { sender: proposal.from }) : "",
    proposal.task_name ? `· ${proposal.task_name}` : "",
  ]
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
    button(t("Send"), async () => {
      const question = proposal.from
        ? t("Send this reply to {sender}?", { sender: proposal.from })
        : t("Send this reply?");
      if (!(await ask(question, { yes: t("Send") }))) throw new Error(t("not sent"));
      await sendReply(proposal.message_id, body.value);
      await invoke("settle_proposal", { id: proposal.id, state: "done", detail: "sent" });
    }, "primary");
    button(t("Edit in compose"), async () => {
      await openReplyInCompose(proposal.message_id, body.value);
      await invoke("settle_proposal", { id: proposal.id, state: "done", detail: "opened in compose" });
    });
  } else {
    button(t("Approve"), async () => {
      await invoke("approve_proposal", { account: state.account, id: proposal.id });
    }, "primary");
  }
  button(t("Reject"), async () => {
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
  say(t("approved {count} — each can be undone from the list with z", { count: done }));
  await refreshTasks();
  await reload({ keepPosition: true });
};

/// Runs the account's tasks. Quiet unless something happened or `loud`.
async function runTasks({ loud = false } = {}) {
  if (state.account === null || state.postbox) return;
  const status = el("tasks-status");
  const channel = progressChannel(([done, total]) => {
    status.textContent = total ? t("asking the model: {done} of {total}", { done, total }) : "";
  });
  status.textContent = t("running…");
  try {
    const runs = await invoke("run_tasks", {
      account: state.account,
      useModel: askModelForUrgency(),
      onProgress: channel,
    });
    const done = runs.reduce((n, r) => n + r.done, 0);
    const proposed = runs.reduce((n, r) => n + r.proposed, 0);
    status.textContent = runs.length ? t("{done} done, {waiting} waiting for you", { done, waiting: proposed }) : "";
    if (done || proposed || loud) {
      say(
        runs.length
          ? t("tasks: {done} done, {waiting} waiting for you", { done, waiting: proposed })
          : t("there are no tasks yet"),
      );
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
  el("task-form-title").textContent = task ? t("Edit “{name}”", { name: task.name }) : t("New task");
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
      ? t("Replies always wait for you: nothing a model writes is sent without you reading it.")
      : kind === "decide"
        ? t("The model reads each message and picks move, archive, trash, mark read, file or a reply — or leaves it.")
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
      out.textContent =
        preview.total === 1
          ? t("One message matches these rules now.")
          : t("{count} messages match these rules now.", { count: preview.total });
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
    say(t("task saved — it runs after every sync"));
    await refreshTasks();
  } catch (err) {
    say(String(err), true);
  }
});

el("task-cancel").onclick = () => closeDialog(taskSheet);
el("task-delete").onclick = async () => {
  if (!(await ask(t("Delete this task? What it did stays done."), { yes: t("Delete"), danger: true }))) return;
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
