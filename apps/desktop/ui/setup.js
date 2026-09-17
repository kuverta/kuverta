// The setup assistant.
//
// Five pages over the window, each of which can be skipped: the models on this
// computer, a Paperless for paper post, and the mail accounts. It opens by
// itself the first time kuverta runs with nothing to show, and from settings
// after that. Everything it finds out, it asks the core; the pages only show
// the answers and pass on what was typed.
//
// Loaded after app.js, whose `invoke`, `el`, `say` and `closeSettings` it uses.

const setup = {
  sheet: el("setup"),
  steps: ["welcome", "models", "paper", "mail", "done"],
  step: 0,
  // What each page came to, for the summary.
  models: null,
  paper: null,
  added: [],
  // Accounts found or looked up, by address, as the mail page shows them.
  found: [],
  // Thunderbird's primary password, when it has one. Kept only while the
  // assistant is open, and only so the core can read the saved passwords.
  primaryPassword: "",
};

async function openSetup() {
  settings.sheet.hidden = true;
  setup.sheet.hidden = false;
  setup.added = [];
  showStep(0);
}

/// Leaves the assistant and puts the window back together, with whatever was
/// added. Finishing or putting it aside both count: it does not open again by
/// itself either way.
async function leaveSetup() {
  await invoke("finish_setup").catch(() => {});
  setup.sheet.hidden = true;
  await closeSettings();
}

el("setup-later").onclick = leaveSetup;
el("settings-setup").onclick = openSetup;

function showStep(index) {
  setup.step = Math.max(0, Math.min(index, setup.steps.length - 1));
  const name = setup.steps[setup.step];
  for (const page of setup.sheet.querySelectorAll("[data-page]")) {
    page.hidden = page.dataset.page !== name;
  }
  setup.sheet.querySelectorAll("#setup-steps li").forEach((item, at) => {
    item.classList.toggle("current", at === setup.step);
    item.classList.toggle("done", at < setup.step);
  });
  el("setup-back").hidden = setup.step === 0;
  el("setup-next").textContent =
    name === "welcome" ? "Start" : name === "done" ? "Open kuverta" : "Continue";
  el("setup-foot-note").textContent = "";

  if (name === "models") refreshModels();
  if (name === "paper") refreshPaper();
  if (name === "mail" && !setup.found.length) scanAccounts();
  if (name === "mail") updateMailFoot();
  if (name === "done") showSummary();
}

el("setup-back").onclick = () => showStep(setup.step - 1);
el("setup-next").onclick = async () => {
  const name = setup.steps[setup.step];
  if (name === "mail") {
    const ok = await addChosenAccounts();
    if (!ok) return;
  }
  if (name === "done") {
    await leaveSetup();
    return;
  }
  showStep(setup.step + 1);
};

// -- small pieces ----------------------------------------------------------------

function node(tag, props = {}, ...children) {
  const element = Object.assign(document.createElement(tag), props);
  element.append(...children.filter((child) => child !== null && child !== undefined));
  return element;
}

/// A line with a mark in front: ✓ done, ✗ missing, … in progress.
function checkLine(state, text, ...extra) {
  const mark = { ok: "✓", bad: "✗", wait: "…" }[state] ?? "•";
  return node(
    "div",
    { className: `check-line ${state}` },
    node("span", { className: "mark", textContent: mark }),
    node("span", { className: "grow", textContent: text }),
    ...extra,
  );
}

function button(label, onclick, { primary = false } = {}) {
  const b = node("button", { type: "button", textContent: label, className: primary ? "primary" : "" });
  b.onclick = async () => {
    b.disabled = true;
    try {
      await onclick(b);
    } catch (err) {
      say(String(err), true);
    } finally {
      b.disabled = false;
    }
  };
  return b;
}

const openLink = (url) => invoke("open_external", { url }).catch((err) => say(String(err), true));

/// A command to copy, for those who would rather type than download.
function commandLine(command) {
  const code = node("code", { textContent: command });
  const copy = button("Copy", async () => {
    await navigator.clipboard.writeText(command);
    say("copied");
  });
  return node("div", { className: "setup-command" }, code, copy);
}

function installHelp(help, what) {
  const parts = [];
  if (help.note) parts.push(node("p", { className: "hint", textContent: help.note }));
  parts.push(node("div", { className: "setup-actions" }, button(`Download ${what}`, () => openLink(help.url), { primary: true })));
  if (help.command) {
    parts.push(node("p", { className: "hint", textContent: "Or in a terminal:" }));
    parts.push(commandLine(help.command));
  }
  return parts;
}

function formatBytes(bytes) {
  if (!bytes) return "";
  const gb = bytes / 1e9;
  return gb >= 1 ? `${gb.toFixed(1)} GB` : `${Math.round(bytes / 1e6)} MB`;
}

// -- models ------------------------------------------------------------------------

const MODEL_PURPOSES = { vision: "reading scanned letters", chat: "sorting mail" };
// Downloads in progress survive the page being redrawn.
const pulling = new Map();

async function refreshModels() {
  const card = el("ollama-card");
  let status;
  try {
    status = await invoke("ollama_status");
  } catch (err) {
    card.replaceChildren(checkLine("bad", String(err)));
    return;
  }
  setup.models = status;
  const lines = [];

  if (!status.local) {
    lines.push(checkLine(status.running ? "ok" : "bad",
      status.running ? `Ollama at ${status.base_url} answers (version ${status.version})`
        : `Ollama at ${status.base_url} is not answering. It is on another computer; start it there.`));
  } else if (!status.installed) {
    lines.push(checkLine("bad", "Ollama is not installed on this computer."));
    lines.push(...installHelp(status.install, "Ollama"));
    lines.push(node("div", { className: "setup-actions" }, button("Check again", refreshModels)));
  } else if (!status.running) {
    lines.push(checkLine("ok", "Ollama is installed."));
    lines.push(checkLine("bad", "It is not running.",
      button("Start Ollama", async (b) => {
        b.textContent = "Starting…";
        await invoke("start_ollama");
        await refreshModels();
      }, { primary: true })));
  } else {
    lines.push(checkLine("ok", `Ollama is running (version ${status.version}).`));
  }

  if (status.running) {
    for (const need of status.needed) {
      lines.push(modelLine(need));
    }
    const missing = status.needed.filter((need) => !need.present && !pulling.has(need.model));
    if (missing.length > 1) {
      lines.push(node("div", { className: "setup-actions" },
        button("Download all", () => Promise.all(missing.map((need) => pullModel(need.model))), { primary: true })));
    }
  }
  card.replaceChildren(...lines);

  const ready = status.running && status.needed.every((need) => need.present);
  el("setup-foot-note").textContent = ready ? "" : "You can continue and come back to this later.";
  el("setup-next").textContent = ready ? "Continue" : "Skip for now";
}

function modelLine(need) {
  const purpose = MODEL_PURPOSES[need.task] ?? need.task;
  if (need.present) return checkLine("ok", `${need.model} for ${purpose} is ready.`);

  const progress = pulling.get(need.model);
  if (progress) {
    const bar = node("progress", { max: 1, value: progress.fraction ?? 0 });
    progress.bar = bar;
    progress.label = node("span", { className: "hint", textContent: progress.text ?? "" });
    return checkLine("wait", `Downloading ${need.model} for ${purpose}`, bar, progress.label);
  }
  const size = need.size ? ` (${need.size})` : "";
  return checkLine("bad", `${need.model} for ${purpose} is not downloaded yet.`,
    button(`Download${size}`, () => pullModel(need.model), { primary: true }));
}

async function pullModel(model) {
  if (pulling.has(model)) return;
  const progress = { fraction: 0, text: "starting…" };
  pulling.set(model, progress);
  await refreshModels();

  const channel = new window.__TAURI__.core.Channel();
  channel.onmessage = (update) => {
    if (update.total) {
      progress.fraction = (update.completed ?? 0) / update.total;
      progress.text = `${formatBytes(update.completed)} of ${formatBytes(update.total)}`;
    } else {
      progress.text = update.status;
    }
    if (progress.bar) progress.bar.value = progress.fraction;
    if (progress.label) progress.label.textContent = progress.text;
  };
  try {
    await invoke("pull_model", { model, onProgress: channel });
    say(`${model} is ready`);
  } catch (err) {
    say(`${model}: ${err}`, true);
  } finally {
    pulling.delete(model);
    if (setup.steps[setup.step] === "models") await refreshModels();
  }
}

// -- paper -------------------------------------------------------------------------

const connectForm = el("paper-connect");
const installForm = el("paper-install-form");

function choosePaper(choice) {
  setup.sheet.querySelectorAll("[data-paper]").forEach((b) => b.classList.toggle("active", b.dataset.paper === choice));
  connectForm.hidden = choice !== "connect";
  el("paper-install").hidden = choice !== "install";
  if (choice === "install") refreshDocker();
  if (choice === "skip") showStep(setup.step + 1);
}

setup.sheet.querySelectorAll("[data-paper]").forEach((b) => {
  b.onclick = () => choosePaper(b.dataset.paper);
});

async function refreshPaper() {
  const card = el("paper-found");
  card.replaceChildren(node("p", { className: "hint", textContent: "Looking for Paperless…" }));
  let found = [];
  try {
    found = await invoke("find_paperless");
  } catch (err) {
    card.replaceChildren(checkLine("bad", String(err)));
    return;
  }
  const lines = [];
  for (const place of found) {
    if (place.configured) {
      lines.push(checkLine(place.answering ? "ok" : "bad",
        place.answering ? `${place.base_url} is connected.` : `${place.base_url} is connected but not answering.`));
    } else if (place.answering) {
      lines.push(checkLine("ok", `Paperless answers at ${place.base_url}.`,
        button("Use this one", () => {
          choosePaper("connect");
          connectForm.base_url.value = place.base_url;
          connectForm.username.focus();
        }, { primary: true })));
    } else if (place.installed_by_kuverta) {
      lines.push(checkLine("bad", `The Paperless kuverta installed (${place.base_url}) is not running.`,
        button("Start it", () => {
          choosePaper("install");
        })));
    }
  }
  if (!lines.length) {
    lines.push(checkLine("bad", "No Paperless found on this computer or at paperless.local."));
    lines.push(node("p", { className: "hint", textContent: "Connect to one elsewhere on your network, or install one here with Docker." }));
  }
  card.replaceChildren(...lines);
  const connected = found.some((place) => place.configured);
  el("setup-next").textContent = connected ? "Continue" : "Skip for now";
}

function syncSetupSelector() {
  const label = connectForm.querySelector("[data-selector-value]");
  label.hidden = connectForm.selector_kind.value === "everything";
}
connectForm.selector_kind.addEventListener("change", syncSetupSelector);

connectForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  const result = el("paper-connect-result");
  const submit = connectForm.querySelector("button[type=submit]");
  const f = Object.fromEntries(new FormData(connectForm).entries());
  const input = {
    id: null,
    label: f.label.trim() || "Post",
    base_url: f.base_url.trim(),
    selector_kind: f.selector_kind,
    selector_value: f.selector_kind === "everything" ? null : f.selector_value.trim() || null,
  };
  submit.disabled = true;
  result.className = "setup-result";
  result.textContent = "Connecting…";
  try {
    const report = await invoke("connect_paperless", {
      input,
      username: f.username.trim() || null,
      password: f.password || null,
      token: f.token || null,
    });
    connectForm.password.value = "";
    connectForm.token.value = "";
    const notes = report.notes.length ? ` ${report.notes.join(" ")}` : "";
    result.className = "setup-result ok";
    result.textContent = `Connected: ${report.documents_matching} of ${report.documents_total} documents belong here.${notes}`;
    setup.paper = input.base_url;
    await refreshPaper();
  } catch (err) {
    result.className = "setup-result bad";
    result.textContent = String(err);
  } finally {
    submit.disabled = false;
  }
});

async function refreshDocker() {
  const card = el("docker-card");
  card.replaceChildren(node("p", { className: "hint", textContent: "Looking for Docker…" }));
  const status = await invoke("docker_status").catch((err) => ({ error: String(err) }));
  const lines = [];
  if (status.error) {
    lines.push(checkLine("bad", status.error));
  } else if (!status.installed) {
    lines.push(checkLine("bad", "Docker is not installed. Paperless runs in it."));
    lines.push(...installHelp(status.install, "Docker"));
    lines.push(node("div", { className: "setup-actions" }, button("Check again", refreshDocker)));
  } else if (!status.running) {
    lines.push(checkLine("ok", "Docker is installed."));
    lines.push(checkLine("bad", status.problem ? `It is not running: ${status.problem}` : "It is not running."));
    lines.push(node("div", { className: "setup-actions" },
      button("Start Docker", async () => {
        await invoke("start_docker");
        say("Docker is starting; check again in a moment");
      }, { primary: true }),
      button("Check again", refreshDocker)));
  } else {
    lines.push(checkLine("ok", `Docker is running (version ${status.version}).`));
  }
  card.replaceChildren(...lines);
  installForm.hidden = !status.running;
}

installForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  const f = Object.fromEntries(new FormData(installForm).entries());
  const log = el("paper-install-log");
  const result = el("paper-install-result");
  const submit = installForm.querySelector("button[type=submit]");
  submit.disabled = true;
  log.hidden = false;
  log.textContent = "";
  result.className = "setup-result";
  result.textContent = "Installing — this takes a few minutes the first time.";

  const lines = [];
  const channel = new window.__TAURI__.core.Channel();
  channel.onmessage = (line) => {
    lines.push(line);
    if (lines.length > 200) lines.shift();
    log.textContent = lines.join("\n");
    log.scrollTop = log.scrollHeight;
  };
  try {
    await invoke("install_paperless", {
      install: { username: f.username.trim(), password: f.password, on_network: Boolean(f.on_network) },
      label: f.label,
      onOutput: channel,
    });
    result.className = "setup-result ok";
    result.textContent = `Paperless is running and connected. Its web page is where you upload and manage documents; sign in as ${f.username.trim()}.`;
    installForm.password.value = "";
    setup.paper = "installed";
    await refreshPaper();
  } catch (err) {
    result.className = "setup-result bad";
    result.textContent = String(err);
  } finally {
    submit.disabled = false;
  }
});

// -- mail --------------------------------------------------------------------------

async function scanAccounts() {
  const sources = el("import-sources");
  sources.replaceChildren(node("p", { className: "hint", textContent: "Looking for accounts in your other mail programs…" }));
  let scan;
  try {
    scan = await invoke("import_accounts", { primaryPassword: setup.primaryPassword || null });
  } catch (err) {
    sources.replaceChildren(checkLine("bad", String(err)));
    return;
  }
  const lines = scan.sources.map((source) => {
    const state = source.state === "found" ? "ok" : source.state === "not_permitted" || source.state === "failed" ? "bad" : "wait";
    const extra = [];
    if (source.action_url) extra.push(button("Open System Settings", () => openLink(source.action_url)));
    if (source.state === "not_permitted") extra.push(button("Look again", scanAccounts));
    const line = checkLine(state, `${source.name}: ${source.state === "not_permitted" ? "not allowed to look" : source.detail}`, ...extra);
    if (source.state === "not_permitted") {
      return node("div", {}, line, node("p", { className: "hint", textContent: source.detail }));
    }
    return line;
  });
  if (!scan.sources.length) lines.push(checkLine("wait", "No other mail programs found."));

  // Thunderbird's saved passwords are behind its primary password: with it,
  // nothing has to be typed here at all.
  if (scan.passwords_locked) {
    const field = node("input", { type: "password", autocomplete: "off", placeholder: "Thunderbird's primary password" });
    const unlock = button("Use it", async () => {
      setup.primaryPassword = field.value;
      await scanAccounts();
      renderFound();
    }, { primary: true });
    lines.push(node("div", { className: "setup-actions" }, field, unlock));
    lines.push(node("p", { className: "hint", textContent: "Thunderbird keeps its saved passwords behind a primary password. Enter it and kuverta can take them over; otherwise enter each password below." }));
  }
  sources.replaceChildren(...lines);

  // Ticked from the start only when there are a few to add as they are; a
  // long list is for choosing from.
  const ready = scan.accounts.filter((account) =>
    account.complete && !account.already_added && account.input.auth_method === "app_password");
  for (const account of scan.accounts) {
    addFound(account, { checked: ready.length <= 3 && ready.includes(account) });
  }
  renderFound();
}

function addFound(account, { checked }) {
  const known = setup.found.find((entry) => entry.account.input.email.toLowerCase() === account.input.email.toLowerCase());
  if (known) {
    known.account = account;
    if (checked) known.checked = true;
    return known;
  }
  const entry = { account, checked: checked && !account.already_added, status: null, statusOk: false, card: null };
  setup.found.push(entry);
  return entry;
}

el("lookup-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const form = event.target;
  const email = form.email.value.trim();
  const submit = form.querySelector("button");
  submit.disabled = true;
  submit.textContent = "Looking…";
  try {
    const account = await invoke("lookup_account", { email });
    const entry = addFound(account, { checked: true });
    renderFound();
    form.email.value = "";
    entry.card?.querySelector("input[name=password]")?.focus();
  } catch (err) {
    say(String(err), true);
  } finally {
    submit.disabled = false;
    submit.textContent = "Find settings";
  }
});

function describeServers(input) {
  if (!input.imap_host) return "servers unknown — enter them below";
  const smtp = input.smtp_host ? `, sends via ${input.smtp_host}:${input.smtp_port}` : ", cannot send (no outgoing server)";
  return `${input.imap_host}:${input.imap_port} (${input.imap_security})${smtp}`;
}

function field(label, name, value, props = {}) {
  const input = node("input", { name, value: value ?? "", spellcheck: false, ...props });
  return node("label", {}, label, input);
}

function securitySelect(name, value) {
  const select = node("select", { name });
  for (const [v, text] of [["tls", "TLS"], ["starttls", "STARTTLS"]]) {
    select.append(node("option", { value: v, textContent: text, selected: v === value }));
  }
  return node("label", {}, "Security", select);
}

function renderFound() {
  const list = el("import-accounts");
  list.replaceChildren(...setup.found.map(accountCard));
  updateMailFoot();
}

function accountCard(entry) {
  const { account } = entry;
  const input = account.input;
  const card = node("div", { className: `import-account${account.already_added ? " added" : ""}` });
  entry.card = card;

  const check = node("input", { type: "checkbox", checked: entry.checked, disabled: account.already_added });
  check.onchange = () => {
    entry.checked = check.checked;
    fields.hidden = !check.checked;
    updateMailFoot();
  };
  const who = account.full_name ? `${account.full_name} <${input.email}>` : input.email;
  const from = account.already_added ? "already in kuverta" : `from ${account.source}`;
  card.append(node("label", { className: "import-head" }, check,
    node("span", { className: "who", textContent: who }),
    node("span", { className: "from", textContent: from })));
  card.append(node("div", { className: "servers", textContent: describeServers(input) }));

  const fields = node("div", { className: "fields setup-form" });
  fields.hidden = !entry.checked || account.already_added;
  // Holds the password field, hidden while the found password is used.
  let typed;

  if (account.password_known) {
    // Nothing to type: the password comes from the program it was found in,
    // and goes straight into the keychain.
    const use = node("input", { type: "checkbox", checked: true, name: "use_found_password" });
    use.onchange = () => {
      typed.hidden = use.checked;
    };
    fields.append(node("label", { className: "setup-check" }, use, `Use the password saved in ${account.source}`));
  }
  typed = node("div", {});
  typed.hidden = account.password_known;
  typed.append(field(input.auth_method === "oauth2" ? "Password (only if you switch to a password below)" : "Password", "password", "", { type: "password", autocomplete: "off" }));
  fields.append(typed);
  if (account.password_help) {
    fields.append(node("p", { className: "hint" },
      account.password_help.text, " ",
      button("Open", () => openLink(account.password_help.url))));
  }

  // The servers and sign-in, for when what was found is not quite right.
  const details = node("details", { open: !account.complete });
  details.append(node("summary", { textContent: "Servers and sign-in" }));
  details.append(node("div", { className: "setup-row3" },
    field("IMAP server", "imap_host", input.imap_host),
    field("Port", "imap_port", input.imap_port, { type: "number" }),
    securitySelect("imap_security", input.imap_security)));
  details.append(node("div", { className: "setup-row3" },
    field("SMTP server", "smtp_host", input.smtp_host),
    field("Port", "smtp_port", input.smtp_port ?? 465, { type: "number" }),
    securitySelect("smtp_security", input.smtp_security ?? "tls")));
  details.append(node("div", { className: "setup-row" },
    field("User name", "username", input.username ?? "", { placeholder: "same as the address" }),
    field("Name in the sidebar", "label", input.label)));
  const method = node("select", { name: "auth_method" });
  method.append(node("option", { value: "app_password", textContent: "Password or app password", selected: input.auth_method !== "oauth2" }));
  method.append(node("option", { value: "oauth2", textContent: "OAuth2 (sign in with kuverta login)", selected: input.auth_method === "oauth2" }));
  const clientId = field("OAuth2 client id", "oauth_client_id", input.oauth_client_id ?? "");
  clientId.hidden = input.auth_method !== "oauth2";
  method.onchange = () => {
    clientId.hidden = method.value !== "oauth2";
  };
  details.append(node("div", { className: "setup-row" }, node("label", {}, "Sign in with", method), clientId));
  fields.append(details);
  card.append(fields);

  if (account.notes.length) {
    card.append(node("div", { className: "notes", textContent: account.notes.join(" ") }));
  }
  card.append(node("div", {
    className: `status ${entry.status ? (entry.statusOk ? "ok" : "bad") : ""}`,
    textContent: entry.status ?? "",
  }));
  return card;
}

/// The account as the card now says, ready for `save_account`.
function cardInput(entry) {
  const card = entry.card;
  const value = (name) => card.querySelector(`[name="${name}"]`)?.value ?? "";
  const base = entry.account.input;
  const smtpHost = value("smtp_host").trim();
  const method = value("auth_method") || base.auth_method;
  return {
    ...base,
    id: null,
    label: value("label").trim() || base.email,
    username: value("username").trim() || null,
    imap_host: value("imap_host").trim(),
    imap_port: Number(value("imap_port")) || 993,
    imap_security: value("imap_security") || "tls",
    smtp_host: smtpHost || null,
    smtp_port: smtpHost ? Number(value("smtp_port")) || 465 : null,
    smtp_security: smtpHost ? value("smtp_security") || "tls" : null,
    auth_method: method,
    oauth_provider: method === "oauth2" ? base.oauth_provider ?? "microsoft" : null,
    oauth_client_id: method === "oauth2" ? value("oauth_client_id").trim() || null : null,
    excluded_folders: base.excluded_folders ?? [],
  };
}

function chosen() {
  return setup.found.filter((entry) => entry.checked && !entry.account.already_added && !entry.statusOk);
}

function updateMailFoot() {
  if (setup.steps[setup.step] !== "mail") return;
  const count = chosen().length;
  el("setup-next").textContent = count ? `Add ${count} account${count === 1 ? "" : "s"}` : setup.added.length ? "Continue" : "Skip for now";
}

/// Saves, stores the password for, and checks each chosen account. True when
/// all of them worked, so the assistant moves on; a failure stays on the page
/// with its reason.
async function addChosenAccounts() {
  const entries = chosen();
  if (!entries.length) return true;
  const next = el("setup-next");
  next.disabled = true;
  let allOk = true;

  for (const entry of entries) {
    const input = cardInput(entry);
    const useFound = entry.card.querySelector("[name=use_found_password]")?.checked ?? false;
    const password = entry.card.querySelector("[name=password]").value;
    const status = entry.card.querySelector(".status");
    const show = (text, ok) => {
      entry.status = text;
      entry.statusOk = ok;
      status.className = `status ${ok ? "ok" : "bad"}`;
      status.textContent = text;
    };

    if (input.auth_method === "app_password" && !password && !useFound) {
      show("Enter the password first.", false);
      allOk = false;
      continue;
    }
    status.className = "status";
    status.textContent = "Checking…";
    let saved = false;
    try {
      const settingsNow = await invoke("account_settings");
      const existing = settingsNow.find((a) => a.email.toLowerCase() === input.email.toLowerCase());
      input.id = existing ? existing.id : null;
      if (useFound && input.auth_method !== "oauth2") {
        // Saved and given its password in one call: the password never
        // crosses into this window.
        await invoke("save_account_with_found_password", {
          input,
          primaryPassword: setup.primaryPassword || null,
        });
        saved = true;
      } else {
        await invoke("save_account", { input });
        saved = true;
        if (input.auth_method === "oauth2") {
          show(`Added. Sign in once in a terminal: kuverta login --email ${input.email}`, true);
          setup.added.push(input.email);
          continue;
        }
        await invoke("set_password", { email: input.email, password });
      }
      const report = await invoke("verify_account", { email: input.email });
      if (!report.imap_ok) {
        show(`Could not sign in: ${report.imap_error}`, false);
        allOk = false;
        continue;
      }
      const sending = report.smtp_target
        ? report.smtp_ok ? " Sending works." : ` Sending did not work: ${report.smtp_error}`
        : "";
      show(`Signed in — ${report.total_messages} messages.${sending}`, true);
      entry.card.querySelector("[name=password]").value = "";
      setup.added.push(input.email);
    } catch (err) {
      show(saved ? `Saved, but: ${err}` : String(err), false);
      allOk = false;
    }
  }
  next.disabled = false;
  updateMailFoot();
  if (!allOk) el("setup-foot-note").textContent = "Fix the accounts marked in red, untick them, or continue without them.";
  return allOk;
}

// -- finish ------------------------------------------------------------------------

async function showSummary() {
  const list = el("setup-summary");
  const lines = [];
  const models = setup.models ?? (await invoke("ollama_status").catch(() => null));
  if (models && models.running && models.needed.every((need) => need.present)) {
    lines.push(checkLine("ok", "Models are ready on this computer."));
  } else {
    lines.push(checkLine("bad", "Models are not ready — scanned letters cannot be read yet. Settings → Setup assistant."));
  }
  const postboxes = await invoke("paper_mailboxes").catch(() => []);
  lines.push(postboxes.length
    ? checkLine("ok", `Paper mail: ${postboxes.map((p) => p.label).join(", ")}.`)
    : checkLine("wait", "No paper mail connected."));
  const accounts = await invoke("accounts").catch(() => []);
  lines.push(accounts.length
    ? checkLine("ok", `${accounts.length} mail account${accounts.length === 1 ? "" : "s"}.`)
    : checkLine("wait", "No mail accounts yet — add them in settings."));
  list.replaceChildren(...lines);

  // New accounts start downloading now. A first sync can take a while, so it
  // goes on after the assistant is closed.
  const note = el("setup-sync-note");
  const fresh = [...new Set(setup.added)];
  if (!fresh.length) {
    note.textContent = "";
    return;
  }
  note.textContent = `Downloading mail for ${fresh.length} account${fresh.length === 1 ? "" : "s"} — you can start using kuverta meanwhile.`;
  setup.added = [];
  syncInTurn(fresh);
}

/// One account after another: each sync writes to the store, and two at once
/// would only wait on each other.
///
/// The note under the summary becomes a progress bar while this runs. A first
/// sync downloads the whole mailbox, so this is the longest wait in the
/// assistant, and the one most worth showing.
async function syncInTurn(emails) {
  const note = el("setup-sync-note");
  const bar = node("progress", { max: 1, value: 0 });
  const label = node("span", { textContent: "" });
  note.replaceChildren(label, bar);

  let done = 0;
  for (const email of emails) {
    const which = emails.length > 1 ? ` (${done + 1} of ${emails.length})` : "";
    label.textContent = `Downloading mail for ${email}${which} — starting…`;
    const channel = new window.__TAURI__.core.Channel();
    channel.onmessage = (at) => {
      if (at.fraction === null || at.fraction === undefined) return;
      bar.value = at.fraction;
      const within = at.messages_total ? `, ${at.messages_done} of ${at.messages_total}` : "";
      label.textContent =
        `Downloading mail for ${email}${which} — ${Math.round(at.fraction * 100)}%: ${at.folder}${within} `;
    };
    try {
      const summary = await invoke("sync", { email, onProgress: channel });
      say(`${email}: ${summary?.inserted ?? 0} messages downloaded`);
    } catch (err) {
      say(`${email}: ${err}`, true);
    }
    done += 1;
  }

  note.textContent = `Mail for ${done} account${done === 1 ? "" : "s"} is downloaded.`;
}
