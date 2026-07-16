// Smart mailboxes: in the sidebar, in their editor, and brought over from
// Thunderbird and Apple Mail.
//
// The core owns the rules and what they select; this draws them and edits
// them. The editor shows what the rules select while they are being written,
// because a rule that selects nothing looks exactly like a smart mailbox with
// nothing in it yet.

const smartSheet = el("smart-sheet");
const smartForm = el("smart-form");
const smartRules = el("smart-rules");

/// Every field a rule can look at, what it can be compared by, and what kind
/// of value it takes. The order is the menu's.
const SMART_FIELDS = [
  ["from", "Sender", "text"],
  ["to", "Recipient", "text"],
  ["subject", "Subject", "text"],
  ["body", "Message text", "words"],
  ["category", "Category", "category"],
  ["list_id", "Mailing list", "text"],
  ["folder", "Mailbox", "folder"],
  ["unread", "Unread", "yesno"],
  ["has_attachment", "Has an attachment", "yesno"],
  ["can_unsubscribe", "Can be unsubscribed from", "yesno"],
  ["older_than_days", "Older than (days)", "days"],
  ["newer_than_days", "Received in the last (days)", "days"],
];

const SMART_OPS = {
  text: [
    ["contains", "contains"],
    ["not_contains", "does not contain"],
    ["is", "is"],
    ["is_not", "is not"],
    ["begins_with", "begins with"],
    ["ends_with", "ends with"],
  ],
  words: [
    ["contains", "contains"],
    ["not_contains", "does not contain"],
  ],
  category: [
    ["is", "is"],
    ["is_not", "is not"],
  ],
  folder: [
    ["is", "is"],
    ["is_not", "is not"],
    ["contains", "contains"],
  ],
  yesno: [["is", "is"]],
  days: [["is", "is"]],
};

const CATEGORY_NAMES = ["personal", "newsletter", "marketing", "transactional", "notification", "unknown"];

/// The mailbox being edited: `{ id, account }`, id null for a new one.
const smartEditing = { id: null, account: null };

ICONS.smart = "M3.5 4.5h13l-5 6v4.2l-3 1.8v-6z";

// -- the sidebar ---------------------------------------------------------------

async function renderSmartMailboxes() {
  const nav = el("smart");
  if (state.account === null || state.postbox) {
    nav.textContent = "";
    return;
  }
  const account = state.account;
  let mailboxes = [];
  try {
    mailboxes = await invoke("smart_mailboxes", { account });
  } catch {
    mailboxes = [];
  }
  if (state.account !== account) return; // moved on while asking
  state.smartMailboxes = mailboxes;
  nav.textContent = "";
  for (const mailbox of mailboxes) {
    const item = navItem({
      label: mailbox.name,
      icon: "smart",
      count: mailbox.total,
      unread: mailbox.unread,
      title: t("{rules} — {unread} unread of {total}", {
        rules: describeQuery(mailbox.query),
        unread: mailbox.unread,
        total: mailbox.total,
      }),
      active: state.view === "mail" && state.filter.smart === mailbox.id,
      onClick: async () => {
        const next = state.filter.smart === mailbox.id ? null : mailbox.id;
        state.view = "mail";
        state.filter = { ...state.filter, smart: next, folder: null, category: null };
        await reload();
      },
    });
    item.addEventListener("contextmenu", (event) => {
      event.preventDefault();
      showMenu(event, [
        [t("Edit…"), () => openSmartEditor(mailbox)],
        null,
        [t("Delete “{name}”", { name: mailbox.name }), () => deleteSmart(mailbox), { danger: true }],
      ]);
    });
    nav.append(item);
  }
}

/// The rules in a line, for a tooltip: "Sender ends with @shop.example and
/// Unread is yes".
function describeQuery(query) {
  const joiner = query.match_all ? ` ${t("and")} ` : ` ${t("or")} `;
  return query.rules
    .map((rule) => {
      const field = SMART_FIELDS.find(([key]) => key === rule.field);
      const ops = SMART_OPS[field?.[2] ?? "text"];
      const op = ops.find(([key]) => key === rule.op)?.[1] ?? rule.op;
      return t("{field} {op} {value}", { field: t(field?.[1] ?? rule.field), op: t(op), value: rule.value });
    })
    .join(joiner);
}

async function deleteSmart(mailbox) {
  if (
    !confirm(
      t("Delete the smart mailbox “{name}”? No mail is deleted — it only stops gathering it.", { name: mailbox.name }),
    )
  )
    return;
  try {
    await invoke("delete_smart_mailbox", { id: mailbox.id });
    if (state.filter.smart === mailbox.id) state.filter = { ...state.filter, smart: null };
    say(t("deleted {name}", { name: mailbox.name }));
    await reload();
    if (!SETTINGS_PAGES.smart.hidden) fillSmartPage();
  } catch (err) {
    say(String(err), true);
  }
}

// -- the editor ----------------------------------------------------------------

/// Opens the editor on a smart mailbox, a suggestion (`{ name, query }` with
/// no id), or nothing for a new one.
function openSmartEditor(mailbox = null) {
  if (state.account === null) {
    say(t("choose a mail account first"), true);
    return;
  }
  smartEditing.id = mailbox?.id ?? null;
  smartEditing.account = mailbox?.account_id ?? state.account;
  el("smart-form-title").textContent = smartEditing.id
    ? t("Edit “{name}”", { name: mailbox.name })
    : t("New smart mailbox");
  el("smart-delete").hidden = smartEditing.id === null;
  smartForm.name.value = mailbox?.name ?? "";
  smartForm.match_all.value = mailbox?.query?.match_all === false ? "any" : "all";
  smartRules.textContent = "";
  const rules = mailbox?.query?.rules?.length
    ? mailbox.query.rules
    : [{ field: "from", op: "contains", value: "" }];
  for (const rule of rules) smartRules.append(ruleRow(rule));
  smartSheet.hidden = false;
  (smartForm.name.value ? smartRules.querySelector("input, select") : smartForm.name)?.focus();
  previewSoon();
}

function ruleRow(rule) {
  const row = document.createElement("div");
  row.className = "rule";

  const field = document.createElement("select");
  field.name = "field";
  for (const [key, label] of SMART_FIELDS) field.append(new Option(t(label), key));
  field.value = rule.field;

  const op = document.createElement("select");
  op.name = "op";

  const slot = document.createElement("span");
  slot.className = "value-slot";

  const remove = document.createElement("button");
  remove.type = "button";
  remove.className = "remove";
  remove.title = t("Remove this rule");
  remove.textContent = "×";
  remove.onclick = () => {
    row.remove();
    if (!smartRules.children.length) smartRules.append(ruleRow({ field: "from", op: "contains", value: "" }));
    previewSoon();
  };

  // The comparison and the value depend on the field, and are rebuilt when it
  // changes — keeping what was typed when the new field can take it.
  const fill = (wanted) => {
    const kind = SMART_FIELDS.find(([key]) => key === field.value)[2];
    op.textContent = "";
    for (const [key, label] of SMART_OPS[kind]) op.append(new Option(t(label), key));
    op.value = SMART_OPS[kind].some(([key]) => key === wanted.op) ? wanted.op : SMART_OPS[kind][0][0];
    // Kept in the grid when there is nothing to choose, so the columns line up.
    op.style.visibility = kind === "days" ? "hidden" : "";
    op.disabled = kind === "yesno";

    let value;
    if (kind === "category") {
      value = document.createElement("select");
      for (const name of CATEGORY_NAMES) value.append(new Option(t(name), name));
      value.value = CATEGORY_NAMES.includes(wanted.value) ? wanted.value : "newsletter";
    } else if (kind === "yesno") {
      value = document.createElement("select");
      value.append(new Option(t("yes"), "yes"), new Option(t("no"), "no"));
      value.value = ["no", "false", "0"].includes(String(wanted.value).toLowerCase()) ? "no" : "yes";
    } else {
      value = document.createElement("input");
      value.spellcheck = false;
      if (kind === "days") {
        value.type = "number";
        value.min = "0";
        value.placeholder = "30";
        value.value = /^\d+$/.test(wanted.value ?? "") ? wanted.value : "";
      } else {
        value.value = wanted.value ?? "";
        value.placeholder = { from: "anna@example.de", to: "me@example.de", subject: t("invoice"), body: t("unsubscribe"), list_id: "news.example", folder: t("Archive|a folder's name") }[field.value] ?? "";
        if (kind === "folder") {
          value.setAttribute("list", "smart-folder-names");
        }
      }
    }
    value.name = "value";
    value.addEventListener("input", previewSoon);
    value.addEventListener("change", previewSoon);
    slot.textContent = "";
    slot.append(value);
  };

  field.addEventListener("change", () => {
    const current = slot.querySelector("[name=value]");
    fill({ op: op.value, value: current?.value ?? "" });
    previewSoon();
  });
  op.addEventListener("change", previewSoon);
  fill(rule);

  row.append(field, op, slot, remove);
  return row;
}

/// The rules as the core takes them.
function smartQuery() {
  const rules = [...smartRules.querySelectorAll(".rule")].map((row) => ({
    field: row.querySelector("[name=field]").value,
    op: row.querySelector("[name=op]").value,
    value: row.querySelector("[name=value]").value.trim(),
  }));
  return { match_all: smartForm.match_all.value === "all", rules };
}

let previewTimer = null;
function previewSoon() {
  clearTimeout(previewTimer);
  previewTimer = setTimeout(previewSmart, 250);
}

async function previewSmart() {
  const count = el("smart-preview-count");
  const rows = el("smart-preview");
  // Rules still being typed are left out rather than shown as an error.
  const query = smartQuery();
  query.rules = query.rules.filter((rule) => rule.value !== "");
  if (!query.rules.length) {
    count.textContent = t("Add a rule to see what it gathers.");
    rows.textContent = "";
    return;
  }
  try {
    const preview = await invoke("preview_smart", { account: smartEditing.account, query });
    count.textContent =
      preview.total === 0
        ? t("Nothing matches these rules yet.")
        : preview.total === 1
          ? t("one message matches")
          : t("{count} messages match", { count: preview.total });
    rows.textContent = "";
    for (const row of preview.rows) {
      const line = document.createElement("div");
      line.className = "preview-row";
      const from = document.createElement("span");
      from.textContent = row.from;
      const subject = document.createElement("span");
      subject.textContent = row.subject;
      const when = document.createElement("span");
      when.className = "when";
      when.textContent = listDate(row.date_utc);
      line.append(from, subject, when);
      rows.append(line);
    }
  } catch (err) {
    count.textContent = String(err);
    rows.textContent = "";
  }
}

smartForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  try {
    const id = await invoke("save_smart_mailbox", {
      input: {
        id: smartEditing.id,
        account_id: smartEditing.account,
        name: smartForm.name.value.trim(),
        query: smartQuery(),
      },
    });
    closeDialog(smartSheet);
    say(smartEditing.id ? t("saved") : t("created {name}", { name: smartForm.name.value.trim() }));
    if (smartEditing.account === state.account) {
      state.filter = { ...state.filter, smart: id, folder: null, category: null };
      await reload();
    }
    if (!SETTINGS_PAGES.smart.hidden) fillSmartPage();
  } catch (err) {
    say(String(err), true);
  }
});

el("smart-rule-add").onclick = () => {
  smartRules.append(ruleRow({ field: "subject", op: "contains", value: "" }));
  smartRules.lastElementChild.querySelector("[name=value]")?.focus();
};
smartForm.match_all.addEventListener("change", previewSoon);
el("smart-cancel").onclick = () => closeDialog(smartSheet);
el("smart-delete").onclick = async () => {
  const mailbox = state.smartMailboxes.find((m) => m.id === smartEditing.id);
  if (!mailbox) return;
  closeDialog(smartSheet);
  await deleteSmart(mailbox);
};
el("smart-add").onclick = () => openSmartEditor();

// Folder names, offered while typing a Mailbox rule.
{
  const names = document.createElement("datalist");
  names.id = "smart-folder-names";
  document.body.append(names);
  smartSheet.addEventListener("focusin", () => {
    names.textContent = "";
    for (const folder of state.folders) names.append(new Option(folder.name));
  });
}

// -- settings: the list, and importing -------------------------------------------

/// What the last look in other programs found.
let smartFound = [];

async function fillSmartPage() {
  const list = el("smart-page-list");
  const account = state.accounts.find((a) => a.id === state.account);
  el("smart-page-account").textContent = account ? t("on {email}", { email: account.email }) : "";
  list.dataset.empty = t("None yet. Make one here, from the + beside Smart mailboxes in the sidebar, or after deleting a message that has look-alikes.");
  list.textContent = "";
  let mailboxes = [];
  try {
    mailboxes = state.account === null ? [] : await invoke("smart_mailboxes", { account: state.account });
  } catch {
    mailboxes = [];
  }
  for (const mailbox of mailboxes) {
    const item = document.createElement("div");
    item.className = "item";
    const text = document.createElement("div");
    text.className = "grow";
    const title = document.createElement("div");
    title.className = "title";
    title.textContent = mailbox.name;
    const sub = document.createElement("div");
    sub.className = "sub";
    sub.textContent = describeQuery(mailbox.query);
    text.append(title, sub);
    const count = document.createElement("span");
    count.className = "chip";
    count.textContent = `${mailbox.total}`;
    const edit = document.createElement("button");
    edit.type = "button";
    edit.textContent = t("Edit");
    edit.onclick = () => openSmartEditor(mailbox);
    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "danger";
    remove.textContent = t("Delete");
    remove.onclick = () => deleteSmart(mailbox);
    item.append(text);
    if (mailbox.source) {
      const from = document.createElement("span");
      from.className = "chip accent";
      from.textContent = mailbox.source === "thunderbird" ? "Thunderbird" : "Apple Mail";
      item.append(from);
    }
    item.append(count, edit, remove);
    list.append(item);
  }
}

async function scanSmartImports() {
  const list = el("smart-import-list");
  const go = el("smart-import-go");
  list.textContent = "";
  list.dataset.empty = t("Looking…");
  go.hidden = true;
  let scan;
  try {
    scan = await invoke("scan_smart_imports");
  } catch (err) {
    list.dataset.empty = String(err);
    return;
  }
  smartFound = scan.found;
  list.dataset.empty = scan.notes.length
    ? scan.notes.join(" · ")
    : t("Neither Thunderbird nor Apple Mail has smart mailboxes on this computer.");
  const account = state.accounts.find((a) => a.id === state.account);

  smartFound.forEach((found, index) => {
    const item = document.createElement("label");
    item.className = "item";
    const tick = document.createElement("input");
    tick.type = "checkbox";
    tick.dataset.index = String(index);
    tick.checked = !found.exists && found.query.rules.length > 0;
    tick.disabled = found.exists || found.query.rules.length === 0;
    const text = document.createElement("div");
    text.className = "grow";
    const title = document.createElement("div");
    title.className = "title";
    title.textContent = found.name;
    const sub = document.createElement("div");
    sub.className = "sub";
    const owner = found.account_id
      ? state.accounts.find((a) => a.id === found.account_id)?.email
      : account?.email;
    const parts = [];
    if (found.query.rules.length) parts.push(describeQuery(found.query));
    else parts.push(t("none of its rules can be carried over"));
    if (owner) parts.push(t("goes to {email}", { email: owner }));
    if (found.exists) parts.push(t("already imported"));
    sub.textContent = parts.join(" · ");
    text.append(title, sub);
    if (found.skipped.length) {
      const lost = document.createElement("div");
      lost.className = "sub bad";
      lost.textContent = t("Left out: {what}", { what: found.skipped.join("; ") });
      text.append(lost);
    }
    const from = document.createElement("span");
    from.className = "chip accent";
    from.textContent = found.source === "thunderbird" ? "Thunderbird" : "Apple Mail";
    item.append(tick, text, from);
    list.append(item);
  });
  if (scan.notes.length && smartFound.length) {
    const note = document.createElement("p");
    note.className = "hint";
    note.textContent = scan.notes.join(" · ");
    list.append(note);
  }
  go.hidden = !smartFound.some((found) => !found.exists && found.query.rules.length);
}

el("smart-import-scan").onclick = scanSmartImports;
el("smart-import-go").onclick = async () => {
  const chosen = [...el("smart-import-list").querySelectorAll("input[type=checkbox]:checked")].map(
    (tick) => smartFound[Number(tick.dataset.index)],
  );
  if (!chosen.length) {
    say(t("tick the ones to bring over"), true);
    return;
  }
  if (state.account === null) {
    say(t("add a mail account first"), true);
    return;
  }
  try {
    const added = await invoke("import_smart_mailboxes", { chosen, fallback: state.account });
    say(added === 1 ? t("imported one smart mailbox") : t("imported {count} smart mailboxes", { count: added }));
    await fillSmartPage();
    await scanSmartImports();
    await renderSmartMailboxes();
  } catch (err) {
    say(String(err), true);
  }
};
el("smart-page-new").onclick = () => openSmartEditor();
SETTINGS_PAGES.smart.addEventListener("show", () => {
  fillSmartPage();
  el("smart-import-list").textContent = "";
  el("smart-import-list").dataset.empty = "";
  el("smart-import-go").hidden = true;
});
