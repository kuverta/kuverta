// The setup assistant.
//
// Five pages over the window, each of which can be skipped: profiles to keep
// private and work mail apart, the models on this computer, a Paperless for
// paper post, and the mail accounts. It opens by
// itself the first time kuverta runs with nothing to show, and from settings
// after that. Everything it finds out, it asks the core; the pages only show
// the answers and pass on what was typed.
//
// Loaded after app.js, whose `invoke`, `el`, `say` and `closeSettings` it uses.

const setup = {
  sheet: el("setup"),
  steps: ["profiles", "models", "paper", "folders", "mail", "done"],
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

/// `step` opens a page other than the first: settings' Import goes straight
/// to the mail accounts, looked for afresh, since what is already in kuverta
/// may have changed since the last look.
async function openSetup(step = "profiles") {
  settings.sheet.hidden = true;
  setup.sheet.hidden = false;
  setup.added = [];
  const at = Math.max(0, setup.steps.indexOf(step));
  if (step === "mail" && setup.found.length) scanAccounts();
  showStep(at);
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
el("settings-setup").onclick = () => openSetup();
el("settings-import-accounts").onclick = () => openSetup("mail");

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
    name === "profiles" ? t("Continue") : name === "done" ? t("Open kuverta") : t("Continue");
  el("setup-foot-note").textContent = "";

  if (name === "profiles") showSetupProfiles();
  if (name === "paper" || name === "mail") fillSetupProfileFields();
  if (name === "models") refreshModels();
  if (name === "paper") refreshPaper();
  if (name === "folders") fillShelf();
  if (name === "mail" && !setup.found.length) scanAccounts();
  if (name === "mail") updateMailFoot();
  if (name === "done") showSummary();
}

/// Without paper post there are no folders to sort it into, so that page is
/// stepped over in both directions.
function nextStep(from, by) {
  let at = from + by;
  while (setup.steps[at] === "folders" && !setup.paper) at += by;
  return at;
}

el("setup-back").onclick = () => showStep(nextStep(setup.step, -1));
el("setup-next").onclick = async () => {
  const name = setup.steps[setup.step];
  if (name === "profiles") {
    const ok = await saveSetupProfiles();
    if (!ok) return;
  }
  if (name === "folders") {
    // Continuing saves them too: nobody should lose a shelf they typed out
    // because they took Continue for the button that keeps it.
    const ok = await saveShelf();
    if (!ok) return;
  }
  if (name === "mail") {
    const ok = await addChosenAccounts();
    if (!ok) return;
  }
  if (name === "done") {
    await leaveSetup();
    return;
  }
  showStep(nextStep(setup.step, 1));
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
  const copy = button(t("Copy"), async () => {
    await navigator.clipboard.writeText(command);
    say(t("copied"));
  });
  return node("div", { className: "setup-command" }, code, copy);
}

function installHelp(help, what) {
  const parts = [];
  if (help.note) parts.push(node("p", { className: "hint", textContent: help.note }));
  parts.push(node("div", { className: "setup-actions" }, button(t("Download {what}", { what }), () => openLink(help.url), { primary: true })));
  if (help.command) {
    parts.push(node("p", { className: "hint", textContent: t("Or in a terminal:") }));
    parts.push(commandLine(help.command));
  }
  return parts;
}

function formatBytes(bytes) {
  if (!bytes) return "";
  const gb = bytes / 1e9;
  return gb >= 1 ? t("{size} GB", { size: gb.toFixed(1) }) : t("{size} MB", { size: Math.round(bytes / 1e6) });
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
      status.running ? t("Ollama at {url} answers (version {version})", { url: status.base_url, version: status.version })
        : t("Ollama at {url} is not answering. It is on another computer; start it there.", { url: status.base_url })));
  } else if (!status.installed) {
    lines.push(checkLine("bad", t("Ollama is not installed on this computer.")));
    lines.push(...installHelp(status.install, "Ollama"));
    lines.push(node("div", { className: "setup-actions" }, button(t("Check again"), refreshModels)));
  } else if (!status.running) {
    lines.push(checkLine("ok", t("Ollama is installed.")));
    lines.push(checkLine("bad", t("It is not running."),
      button(t("Start Ollama"), async (b) => {
        b.textContent = t("Starting…");
        await invoke("start_ollama");
        await refreshModels();
      }, { primary: true })));
  } else {
    lines.push(checkLine("ok", t("Ollama is running (version {version}).", { version: status.version })));
  }

  if (status.running) {
    for (const need of status.needed) {
      lines.push(modelLine(need));
    }
    const missing = status.needed.filter((need) => !need.present && !pulling.has(need.model));
    if (missing.length > 1) {
      lines.push(node("div", { className: "setup-actions" },
        button(t("Download all"), () => Promise.all(missing.map((need) => pullModel(need.model))), { primary: true })));
    }
  }
  card.replaceChildren(...lines);

  const ready = status.running && status.needed.every((need) => need.present);
  el("setup-foot-note").textContent = ready ? "" : t("You can continue and come back to this later.");
  el("setup-next").textContent = ready ? t("Continue") : t("Skip for now");
}

function modelLine(need) {
  const purpose = t(MODEL_PURPOSES[need.task] ?? need.task);
  if (need.present) return checkLine("ok", t("{model} for {purpose} is ready.", { model: need.model, purpose }));

  const progress = pulling.get(need.model);
  if (progress) {
    const bar = node("progress", { max: 1, value: progress.fraction ?? 0 });
    progress.bar = bar;
    progress.label = node("span", { className: "hint", textContent: progress.text ?? "" });
    return checkLine("wait", t("Downloading {model} for {purpose}", { model: need.model, purpose }), bar, progress.label);
  }
  const download = need.size ? t("Download ({size})", { size: need.size }) : t("Download");
  return checkLine("bad", t("{model} for {purpose} is not downloaded yet.", { model: need.model, purpose }),
    button(download, () => pullModel(need.model), { primary: true }));
}

async function pullModel(model) {
  if (pulling.has(model)) return;
  const progress = { fraction: 0, text: t("starting…") };
  pulling.set(model, progress);
  await refreshModels();

  const channel = new window.__TAURI__.core.Channel();
  channel.onmessage = (update) => {
    if (update.total) {
      progress.fraction = (update.completed ?? 0) / update.total;
      progress.text = t("{done} of {total}", { done: formatBytes(update.completed), total: formatBytes(update.total) });
    } else {
      progress.text = update.status;
    }
    if (progress.bar) progress.bar.value = progress.fraction;
    if (progress.label) progress.label.textContent = progress.text;
  };
  try {
    await invoke("pull_model", { model, onProgress: channel });
    say(t("{model} is ready", { model }));
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
  if (choice === "skip") showStep(nextStep(setup.step, 1));
}

setup.sheet.querySelectorAll("[data-paper]").forEach((b) => {
  b.onclick = () => choosePaper(b.dataset.paper);
});

async function refreshPaper() {
  const card = el("paper-found");
  card.replaceChildren(node("p", { className: "hint", textContent: t("Looking for Paperless…") }));
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
        place.answering ? t("{url} is connected.", { url: place.base_url })
          : t("{url} is connected but not answering.", { url: place.base_url })));
    } else if (place.answering) {
      lines.push(checkLine("ok", t("Paperless answers at {url}.", { url: place.base_url }),
        button(t("Use this one"), () => {
          choosePaper("connect");
          connectForm.base_url.value = place.base_url;
          connectForm.username.focus();
        }, { primary: true })));
    } else if (place.installed_by_kuverta) {
      lines.push(checkLine("bad", t("The Paperless kuverta installed ({url}) is not running.", { url: place.base_url }),
        button(t("Start it"), () => {
          choosePaper("install");
        })));
    }
  }
  if (!lines.length) {
    lines.push(checkLine("bad", t("No Paperless found on this computer or at paperless.local.")));
    lines.push(node("p", { className: "hint", textContent: t("Connect to one elsewhere on your network, or install one here with Docker.") }));
  }
  card.replaceChildren(...lines);
  const connected = found.find((place) => place.configured);
  // Coming back to a Paperless set up earlier also leads to the folders.
  if (connected && !setup.paper) setup.paper = connected.base_url;
  el("setup-next").textContent = connected ? t("Continue") : t("Skip for now");
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
  result.textContent = t("Connecting…");
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
    result.textContent = t("Connected: {matching} of {total} documents belong here.",
      { matching: report.documents_matching, total: report.documents_total }) + notes;
    setup.paper = input.base_url;
    await placePostbox(input.base_url, connectForm.profile);
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
  card.replaceChildren(node("p", { className: "hint", textContent: t("Looking for Docker…") }));
  const status = await invoke("docker_status").catch((err) => ({ error: String(err) }));
  const lines = [];
  if (status.error) {
    lines.push(checkLine("bad", status.error));
  } else if (!status.installed) {
    lines.push(checkLine("bad", t("Docker is not installed. Paperless runs in it.")));
    lines.push(...installHelp(status.install, "Docker"));
    lines.push(node("div", { className: "setup-actions" }, button(t("Check again"), refreshDocker)));
  } else if (!status.running) {
    lines.push(checkLine("ok", t("Docker is installed.")));
    lines.push(checkLine("bad", status.problem ? t("It is not running: {problem}", { problem: status.problem }) : t("It is not running.")));
    lines.push(node("div", { className: "setup-actions" },
      button(t("Start Docker"), async () => {
        await invoke("start_docker");
        say(t("Docker is starting; check again in a moment"));
      }, { primary: true }),
      button(t("Check again"), refreshDocker)));
  } else {
    lines.push(checkLine("ok", t("Docker is running (version {version}).", { version: status.version })));
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
  result.textContent = t("Installing — this takes a few minutes the first time.");

  const lines = [];
  const channel = new window.__TAURI__.core.Channel();
  channel.onmessage = (line) => {
    lines.push(line);
    if (lines.length > 200) lines.shift();
    log.textContent = lines.join("\n");
    log.scrollTop = log.scrollHeight;
  };
  try {
    const postbox = await invoke("install_paperless", {
      install: { username: f.username.trim(), password: f.password, on_network: Boolean(f.on_network) },
      label: f.label,
      onOutput: channel,
    });
    result.className = "setup-result ok";
    const made = !f.password.trim();
    result.textContent = made
      ? t("Paperless is running and connected. Its web page is where you upload and manage documents; sign in as {user}. kuverta made the password and put it in the keychain: Settings → the address → Show Paperless sign-in.", { user: f.username.trim() })
      : t("Paperless is running and connected. Its web page is where you upload and manage documents; sign in as {user}.", { user: f.username.trim() });
    installForm.password.value = "";
    setup.paper = "installed";
    await invoke("set_postbox_profile", { postbox, profile: selectedProfile(installForm.profile) }).catch(() => {});
    await refreshPaper();
  } catch (err) {
    result.className = "setup-result bad";
    result.textContent = String(err);
  } finally {
    submit.disabled = false;
  }
});

// -- folders on the shelf ------------------------------------------------------------

/// What most households need, as somewhere to start. Each is a Paperless tag
/// with words that only its letters carry; the words are deliberately narrow,
/// because a word every official letter contains — "Datenschutz", say, which
/// stands in the footer of all of them — would put every letter in that
/// folder.
const SHELF_SUGGESTIONS = [
  { name: "Auto", words: "Kfz, Fahrzeug, Fahrzeughalter, Kennzeichen, TÜV, Hauptuntersuchung, Zulassungsstelle, Kfz-Versicherung, Bußgeldbescheid, Verwarnungsgeld" },
  { name: "Rechnungen", words: "Rechnung, Rechnungsnummer, Rechnungsbetrag, Zahlungserinnerung, Mahnbescheid, Inkasso, Zahlungsziel, Ratenzahlung" },
  { name: "Versicherungen", words: "Versicherungsschein, Versicherungsnummer, Beitragsrechnung, Haftpflicht, Hausratversicherung, Krankenversicherung, Schadenmeldung" },
  { name: "Steuern", words: "Finanzamt, Steuernummer, Steuerbescheid, Einkommensteuer, Steuererklärung, Lohnsteuerbescheinigung, Grundsteuer, ELSTER" },
  { name: "Wohnen", words: "Miete, Mietvertrag, Vermieter, Hausverwaltung, Nebenkostenabrechnung, Betriebskostenabrechnung, Stromrechnung, Zählerstand, Rundfunkbeitrag" },
  { name: "Werbung", words: "Werbung, Gewinnspiel, Gutschein, Rabatt, Sonderangebot, Prospekt, Katalog, Newsletter" },
];

/// The address the folders belong to: the one just set up, or the only one.
async function shelfAddress() {
  const addresses = await invoke("paper_mailboxes").catch(() => []);
  if (!addresses.length) return null;
  const same = (a, b) =>
    String(a).trim().replace(/\/$/, "").toLowerCase() === String(b).trim().replace(/\/$/, "").toLowerCase();
  return addresses.find((box) => setup.paper && same(box.base_url, setup.paper)) ?? addresses[addresses.length - 1];
}

/// The rows on the page, as they are being edited.
let shelf = null;

async function fillShelf() {
  const result = el("shelf-result");
  result.className = "setup-result";
  result.textContent = "";
  const address = await shelfAddress();
  el("shelf-add").disabled = !address;
  el("shelf-save").disabled = !address;
  if (!address) {
    result.className = "setup-result bad";
    result.textContent = t("No paper post connected yet — the page before this one.");
    el("shelf").replaceChildren();
    return;
  }
  if (shelf === null) {
    // What Paperless already has, if anything: setting up again should not
    // throw away folders that are in use.
    const existing = await invoke("paper_folders", { id: address.id }).catch(() => []);
    shelf = existing.length
      ? existing.map((folder) => ({ ...folder }))
      : SHELF_SUGGESTIONS.map((folder) => ({ ...folder, person: false }));
  }
  drawShelf();
}

/// Both lists are the same rows, told apart by `person`: a folder is somewhere
/// paper goes, a person somebody it is for.
function drawShelf() {
  drawRows(el("shelf"), false);
  drawRows(el("household"), true);
}

function drawRows(into, people) {
  into.replaceChildren(
    ...shelf
      .map((folder, index) => ({ folder, index }))
      .filter(({ folder }) => Boolean(folder.person) === people)
      .map(({ folder, index }) => {
        const name = node("input", {
          value: folder.name,
          placeholder: people ? t("Name") : t("Name of the folder"),
          spellcheck: false,
        });
        const words = node("input", {
          value: folder.words ?? "",
          placeholder: people
            ? t("Other spellings of the name (optional)")
            : t("Words, separated by commas (optional)"),
          spellcheck: false,
          className: "shelf-words",
        });
        const remove = node("button", {
          type: "button",
          textContent: "✕",
          title: people ? t("Remove this person") : t("Remove this folder"),
        });
        name.oninput = () => (folder.name = name.value);
        words.oninput = () => (folder.words = words.value);
        remove.onclick = () => {
          shelf.splice(index, 1);
          drawShelf();
        };
        return node("div", { className: "shelf-row" }, name, words, remove);
      }),
  );
  if (people && !into.children.length) {
    into.append(node("p", { className: "hint", textContent: t("Nobody yet — one person's post needs nobody named.") }));
  }
}

function addShelfRow(person) {
  shelf = shelf ?? [];
  shelf.push({ name: "", words: "", person });
  drawShelf();
  const list = el(person ? "household" : "shelf");
  list.lastElementChild?.querySelector("input")?.focus();
}

el("shelf-add").onclick = () => addShelfRow(false);
el("household-add").onclick = () => addShelfRow(true);

el("shelf-save").onclick = saveShelf;

/// Writes the rows to Paperless. False when something was wrong with them, or
/// Paperless would not take them — the page says what, and stays put.
async function saveShelf() {
  const address = await shelfAddress();
  if (!address || shelf === null) return true;
  const folders = (shelf ?? [])
    .map((folder) => ({
      name: folder.name.trim(),
      words: (folder.words ?? "").trim(),
      person: Boolean(folder.person),
    }))
    .filter((folder) => folder.name);
  // A folder and a person may share a name — they are two tags — so each list
  // is checked on its own.
  const names = folders.map((folder) => `${folder.person}:${folder.name.toLowerCase()}`);
  const twiceAt = names.findIndex((name, at) => names.indexOf(name) !== at);
  const twice = twiceAt < 0 ? undefined : folders[twiceAt].name;
  const result = el("shelf-result");
  const complain = (text) => {
    result.className = "setup-result bad";
    result.textContent = text;
  };
  if (twice) {
    complain(t("Two folders are called {name}.", { name: twice }));
    return false;
  }
  // Paperless keeps at most 256 characters of a tag's words.
  const tooLong = folders.find(
    (folder) => wordsForPaperless(folder.words || (folder.person ? folder.name : "")).length > 256,
  );
  if (tooLong) {
    complain(t("Too many words for {name} — leave some out.", { name: tooLong.name }));
    return false;
  }

  el("shelf-save").disabled = true;
  result.className = "setup-result";
  result.textContent = t("Setting the folders up…");
  let saved = true;
  try {
    await invoke("set_paper_folders", { id: address.id, folders });
    result.className = "setup-result ok";
    const people = folders.filter((folder) => folder.person).length;
    result.textContent = people
      ? t("{count} folders and {people} people are set up in Paperless. The scanner takes its list from there.", {
          count: folders.length - people,
          people,
        })
      : t("{count} folders are set up in Paperless. The scanner takes its list from there.", {
          count: folders.length,
        });
  } catch (error) {
    complain(String(error));
    saved = false;
  }
  el("shelf-save").disabled = false;
  return saved;
}

/// The words as Paperless stores them for a tag, which is what its 256
/// characters are counted in: a space between them, a phrase quoted.
function wordsForPaperless(words) {
  return words
    .split(",")
    .map((word) => word.trim())
    .filter(Boolean)
    .map((word) => (word.includes(" ") ? `"${word.replaceAll('"', "")}"` : word))
    .join(" ");
}

// -- mail --------------------------------------------------------------------------

async function scanAccounts() {
  const sources = el("import-sources");
  sources.replaceChildren(node("p", { className: "hint", textContent: t("Looking for accounts in your other mail programs…") }));
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
    if (source.action_url) extra.push(button(t("Open System Settings"), () => openLink(source.action_url)));
    if (source.state === "not_permitted") extra.push(button(t("Look again"), scanAccounts));
    const line = checkLine(state, `${source.name}: ${source.state === "not_permitted" ? t("not allowed to look") : source.detail}`, ...extra);
    if (source.state === "not_permitted") {
      return node("div", {}, line, node("p", { className: "hint", textContent: source.detail }));
    }
    return line;
  });
  if (!scan.sources.length) lines.push(checkLine("wait", t("No other mail programs found.")));

  // Thunderbird's saved passwords are behind its primary password: with it,
  // nothing has to be typed here at all.
  if (scan.passwords_locked) {
    const field = node("input", { type: "password", autocomplete: "off", placeholder: t("Thunderbird's primary password") });
    const unlock = button(t("Use it"), async () => {
      setup.primaryPassword = field.value;
      await scanAccounts();
      renderFound();
    }, { primary: true });
    lines.push(node("div", { className: "setup-actions" }, field, unlock));
    lines.push(node("p", { className: "hint", textContent: t("Thunderbird keeps its saved passwords behind a primary password. Enter it and kuverta can take them over; otherwise enter each password below.") }));
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
  submit.textContent = t("Looking…");
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
    submit.textContent = t("Find settings");
  }
});

function describeServers(input) {
  if (!input.imap_host) return t("servers unknown — enter them below");
  const imap = `${input.imap_host}:${input.imap_port} (${input.imap_security})`;
  return input.smtp_host
    ? t("{imap}, sends via {host}:{port}", { imap, host: input.smtp_host, port: input.smtp_port })
    : t("{imap}, cannot send (no outgoing server)", { imap });
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
  return node("label", {}, t("Security"), select);
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
  const from = account.already_added ? t("already in kuverta") : t("from {source}", { source: account.source });
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
    fields.append(node("label", { className: "setup-check" }, use, t("Use the password saved in {source}", { source: account.source })));
  }
  typed = node("div", {});
  typed.hidden = account.password_known;
  typed.append(field(input.auth_method === "oauth2" ? t("Password (only if you switch to a password below)") : t("Password"), "password", "", { type: "password", autocomplete: "off" }));
  fields.append(typed);
  if (account.password_help) {
    fields.append(node("p", { className: "hint" },
      account.password_help.text, " ",
      button(t("Open"), () => openLink(account.password_help.url))));
  }

  // The servers and sign-in, for when what was found is not quite right.
  const details = node("details", { open: !account.complete });
  details.append(node("summary", { textContent: t("Servers and sign-in") }));
  details.append(node("div", { className: "setup-row3" },
    field(t("IMAP server"), "imap_host", input.imap_host),
    field(t("Port"), "imap_port", input.imap_port, { type: "number" }),
    securitySelect("imap_security", input.imap_security)));
  details.append(node("div", { className: "setup-row3" },
    field(t("SMTP server"), "smtp_host", input.smtp_host),
    field(t("Port"), "smtp_port", input.smtp_port ?? 465, { type: "number" }),
    securitySelect("smtp_security", input.smtp_security ?? "tls")));
  details.append(node("div", { className: "setup-row" },
    field(t("User name"), "username", input.username ?? "", { placeholder: t("same as the address") }),
    field(t("Name in the sidebar"), "label", input.label)));
  const method = node("select", { name: "auth_method" });
  method.append(node("option", { value: "app_password", textContent: t("Password or app password"), selected: input.auth_method !== "oauth2" }));
  method.append(node("option", { value: "oauth2", textContent: t("OAuth2 (sign in with kuverta login)"), selected: input.auth_method === "oauth2" }));
  const clientId = field(t("OAuth2 client id"), "oauth_client_id", input.oauth_client_id ?? "");
  clientId.hidden = input.auth_method !== "oauth2";
  method.onchange = () => {
    clientId.hidden = method.value !== "oauth2";
  };
  details.append(node("div", { className: "setup-row" }, node("label", {}, t("Sign in with"), method), clientId));
  if (state.profiles.length) {
    const profile = node("select", { name: "profile" });
    fillProfileSelect(profile, entry.profile ?? activeProfile() ?? state.profiles[0].id);
    profile.onchange = () => (entry.profile = selectedProfile(profile));
    fields.append(node("label", {}, t("Profile"), profile));
  }
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
  el("setup-next").textContent = count
    ? count === 1 ? t("Add one account") : t("Add {count} accounts", { count })
    : setup.added.length ? t("Continue") : t("Skip for now");
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
      show(t("Enter the password first."), false);
      allOk = false;
      continue;
    }
    status.className = "status";
    status.textContent = t("Checking…");
    let saved = false;
    try {
      const settingsNow = await invoke("account_settings");
      const existing = settingsNow.find((a) => a.email.toLowerCase() === input.email.toLowerCase());
      input.id = existing ? existing.id : null;
      if (useFound && input.auth_method !== "oauth2") {
        // Saved and given its password in one call: the password never
        // crosses into this window.
        const id = await invoke("save_account_with_found_password", {
          input,
          primaryPassword: setup.primaryPassword || null,
        });
        saved = true;
        await placeAccount(id, entry);
      } else {
        const id = await invoke("save_account", { input });
        saved = true;
        await placeAccount(id, entry);
        if (input.auth_method === "oauth2") {
          show(t("Added. Sign in once in a terminal: kuverta login --email {email}", { email: input.email }), true);
          setup.added.push(input.email);
          continue;
        }
        await invoke("set_password", { email: input.email, password });
      }
      const report = await invoke("verify_account", { email: input.email });
      if (!report.imap_ok) {
        show(t("Could not sign in: {error}", { error: report.imap_error }), false);
        allOk = false;
        continue;
      }
      const sending = report.smtp_target
        ? report.smtp_ok ? ` ${t("Sending works.")}` : ` ${t("Sending did not work: {error}", { error: report.smtp_error })}`
        : "";
      show(t("Signed in — {count} messages.", { count: report.total_messages }) + sending, true);
      entry.card.querySelector("[name=password]").value = "";
      setup.added.push(input.email);
    } catch (err) {
      show(saved ? t("Saved, but: {error}", { error: err }) : String(err), false);
      allOk = false;
    }
  }
  next.disabled = false;
  updateMailFoot();
  if (!allOk) el("setup-foot-note").textContent = t("Fix the accounts marked in red, untick them, or continue without them.");
  return allOk;
}

// -- profiles ------------------------------------------------------------------------

/// The profile rows on the first page: the ones there are, then new ones.
async function showSetupProfiles() {
  await loadProfiles();
  const list = el("setup-profiles");
  if (list.dataset.filled) return;
  list.dataset.filled = "1";
  list.textContent = "";
  for (const profile of state.profiles) addSetupProfileRow(profile.name, profile.id);
}

function addSetupProfileRow(name = "", id = null) {
  const list = el("setup-profiles");
  const input = node("input", { value: name, placeholder: t("Name, e.g. Private"), spellcheck: false });
  input.dataset.id = id ?? "";
  const row = node("div", { className: "setup-profile" }, input);
  if (id === null) {
    const remove = node("button", { type: "button", textContent: t("Remove") });
    remove.onclick = () => row.remove();
    row.append(remove);
  }
  list.append(row);
  if (!name) input.focus();
}

el("setup-profile-add").onclick = () => addSetupProfileRow();
for (const chip of setup.sheet.querySelectorAll("[data-suggest]")) {
  chip.onclick = () => {
    const taken = [...el("setup-profiles").querySelectorAll("input")].map((i) => i.value.trim().toLowerCase());
    if (!taken.includes(chip.dataset.suggest.toLowerCase())) addSetupProfileRow(chip.dataset.suggest);
  };
}

/// Saves new profiles and renames; true when all went, so the assistant moves on.
async function saveSetupProfiles() {
  const rows = [...el("setup-profiles").querySelectorAll("input")];
  for (const input of rows) {
    const name = input.value.trim();
    const id = input.dataset.id ? Number(input.dataset.id) : null;
    if (!name) continue;
    try {
      const saved = await invoke("save_profile", { id, name });
      input.dataset.id = String(saved);
    } catch (err) {
      el("setup-foot-note").textContent = String(err);
      input.focus();
      return false;
    }
  }
  await loadProfiles();
  return true;
}

/// The Profile fields on the paper page, shown once there are profiles.
function fillSetupProfileFields() {
  for (const label of setup.sheet.querySelectorAll("[data-profile-field]")) {
    const select = label.querySelector("select");
    fillProfileSelect(select, activeProfile() ?? state.profiles[0]?.id ?? null);
    label.hidden = state.profiles.length === 0;
  }
  // The account cards draw theirs when they are drawn; draw them again so a
  // profile added on the first page is offered.
  // Only when they lack the field: redrawing loses what was typed into them.
  const cards = el("import-accounts");
  if (setup.found.length && state.profiles.length && !cards.querySelector("[name=profile]")) renderFound();
}

async function placeAccount(id, entry) {
  const profile = entry.card.querySelector("[name=profile]");
  if (!profile) return;
  await invoke("set_account_profile", { account: id, profile: selectedProfile(profile) }).catch(() => {});
}

/// The postal address just connected, found by its address, put in a profile.
async function placePostbox(baseUrl, select) {
  if (!state.profiles.length) return;
  const postboxes = await invoke("paper_mailboxes").catch(() => []);
  const postbox = postboxes.filter((p) => p.base_url === baseUrl).pop();
  if (postbox) await invoke("set_postbox_profile", { postbox: postbox.id, profile: selectedProfile(select) }).catch(() => {});
}

// -- finish ------------------------------------------------------------------------

async function showSummary() {
  const list = el("setup-summary");
  const lines = [];
  const models = setup.models ?? (await invoke("ollama_status").catch(() => null));
  if (models && models.running && models.needed.every((need) => need.present)) {
    lines.push(checkLine("ok", t("Models are ready on this computer.")));
  } else {
    lines.push(checkLine("bad", t("Models are not ready — scanned letters cannot be read yet. Settings → Setup assistant.")));
  }
  const postboxes = await invoke("paper_mailboxes").catch(() => []);
  lines.push(postboxes.length
    ? checkLine("ok", t("Paper mail: {list}.", { list: postboxes.map((p) => p.label).join(", ") }))
    : checkLine("wait", t("No paper mail connected.")));
  if (postboxes.length) {
    const address = (await shelfAddress()) ?? postboxes[postboxes.length - 1];
    const folders = await invoke("paper_folders", { id: address.id }).catch(() => []);
    lines.push(folders.some((f) => !f.person)
      ? checkLine("ok", t("Folders on the shelf: {list}.", {
          list: folders.filter((f) => !f.person).map((f) => f.name).join(", "),
        }))
      : checkLine("wait", t("No folders on the shelf — the scanner cannot say where a letter goes.")));
    const people = folders.filter((folder) => folder.person);
    if (people.length) {
      lines.push(checkLine("ok", t("Post here is for: {list}.", { list: people.map((f) => f.name).join(", ") })));
    }
  }
  const accounts = await invoke("accounts").catch(() => []);
  await loadProfiles();
  if (state.profiles.length) {
    lines.push(checkLine("ok", t("Profiles: {list}.", { list: state.profiles.map((p) => `${p.name} (${p.accounts.length + p.postboxes.length})`).join(", ") })));
  }
  lines.push(accounts.length
    ? checkLine("ok", accounts.length === 1 ? t("one mail account.") : t("{count} mail accounts.", { count: accounts.length }))
    : checkLine("wait", t("No mail accounts yet — add them in settings.")));
  list.replaceChildren(...lines);

  // New accounts start downloading now. A first sync can take a while, so it
  // goes on after the assistant is closed.
  const note = el("setup-sync-note");
  const fresh = [...new Set(setup.added)];
  if (!fresh.length) {
    note.textContent = "";
    return;
  }
  note.textContent = fresh.length === 1
    ? t("Downloading mail for one account — you can start using kuverta meanwhile.")
    : t("Downloading mail for {count} accounts — you can start using kuverta meanwhile.", { count: fresh.length });
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
    const many = emails.length > 1;
    const nth = done + 1;
    label.textContent = many
      ? t("Downloading mail for {email} ({nth} of {total}) — starting…", { email, nth, total: emails.length })
      : t("Downloading mail for {email} — starting…", { email });
    const channel = new window.__TAURI__.core.Channel();
    channel.onmessage = (at) => {
      if (at.fraction === null || at.fraction === undefined) return;
      bar.value = at.fraction;
      const vars = {
        email, nth, total: emails.length,
        percent: Math.round(at.fraction * 100),
        folder: at.folder, done: at.messages_done, messages: at.messages_total,
      };
      label.textContent = at.messages_total
        ? many
          ? t("Downloading mail for {email} ({nth} of {total}) — {percent}%: {folder}, {done} of {messages} ", vars)
          : t("Downloading mail for {email} — {percent}%: {folder}, {done} of {messages} ", vars)
        : many
          ? t("Downloading mail for {email} ({nth} of {total}) — {percent}%: {folder} ", vars)
          : t("Downloading mail for {email} — {percent}%: {folder} ", vars);
    };
    try {
      const summary = await invoke("sync", { email, onProgress: channel });
      say(t("{email}: {count} messages downloaded", { email, count: summary?.inserted ?? 0 }));
    } catch (err) {
      say(`${email}: ${err}`, true);
    }
    done += 1;
  }

  note.textContent = done === 1
    ? t("Mail for one account is downloaded.")
    : t("Mail for {count} accounts is downloaded.", { count: done });
}
