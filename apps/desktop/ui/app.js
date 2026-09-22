// The triage surface.
//
// The virtualizer is the one from the spike and works the same way: a fixed
// pool of DOM nodes is recycled as the viewport moves, so the node count stays
// proportional to the window rather than to the mailbox. What changed is where
// the rows come from. The spike held all of them in memory; here the list owns
// only the pages it has asked for, because the bridge that made the spike
// worth running is the same bridge a whole mailbox would have to cross.

const { invoke } = window.__TAURI__.core;

const ROW_HEIGHT = 74;
const OVERSCAN = 8;
// One page is several screenfuls, so scrolling at a normal speed stays ahead
// of the fetch without asking for much that is never shown.
const PAGE = 200;

const el = (id) => document.getElementById(id);

/// The list with nothing narrowing it. A smart mailbox is one more filter, and
/// like a folder it is replaced rather than combined when another is chosen.
const NO_FILTER = Object.freeze({ category: null, unreadOnly: false, folder: null, smart: null });

/// Sidebar glyphs, as inline SVG.
///
/// Drawn rather than pulled from a font: the webview has no access to SF
/// Symbols, and a pictogram font would be a download for six shapes.
const ICONS = {
  inbox: "M3 13h3.5l1.2 2h4.6l1.2-2H17M3 13l2.4-7.6A1 1 0 016.35 4.7h7.3a1 1 0 01.95.7L17 13v2.3a1 1 0 01-1 1H4a1 1 0 01-1-1z",
  drafts: "M4 16h12M13.1 3.9l3 3L7.6 15.4l-3.9.9.9-3.9z",
  sent: "M17 3L2.5 9.2l5.6 2.2 2.2 5.6zM17 3l-8.9 8.4",
  archive: "M3 6.5h14M4.5 6.5v9a1 1 0 001 1h9a1 1 0 001-1v-9M3 6.5l1.3-2.2a1 1 0 01.86-.5h9.68a1 1 0 01.86.5L17 6.5M8 10h4",
  junk: "M10 3l7.5 13H2.5zM10 8v3.5M10 13.6v.1",
  trash: "M3.5 5.5h13M8 5.5V4a1 1 0 011-1h2a1 1 0 011 1v1.5M5 5.5v10a1 1 0 001 1h8a1 1 0 001-1v-10M8.5 8.5v5M11.5 8.5v5",
  folder: "M3 15.5v-10a1 1 0 011-1h3.8a1 1 0 01.78.37l1.04 1.26a1 1 0 00.78.37H16a1 1 0 011 1v8a1 1 0 01-1 1H4a1 1 0 01-1-1z",
  all: "M3 5.5h14M3 10h14M3 14.5h9",
  tag: "M3 3h6.3a1 1 0 01.7.3l6.7 6.7a1 1 0 010 1.4l-5.3 5.3a1 1 0 01-1.4 0L3.3 10A1 1 0 013 9.3zM6.5 6.5v.01",
  post: "M3 6h14v8.5a1 1 0 01-1 1H4a1 1 0 01-1-1zM3 6.5l7 5 7-5",
};

/// Which glyph a folder gets, from its special-use attribute and then its name.
///
/// The attribute first, because it is what the server actually asserts; the
/// name only as a fallback for the many servers that assert nothing.
function iconFor(folder) {
  const byUse = {
    "\\Drafts": "drafts",
    "\\Sent": "sent",
    "\\Archive": "archive",
    "\\All": "archive",
    "\\Junk": "junk",
    "\\Trash": "trash",
  }[folder.special_use];
  if (byUse) return byUse;
  if (folder.name.toUpperCase() === "INBOX") return "inbox";
  return "folder";
}

function iconSvg(name) {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("class", "icon");
  svg.setAttribute("viewBox", "0 0 20 20");
  svg.setAttribute("fill", "none");
  svg.setAttribute("stroke", "currentColor");
  svg.setAttribute("stroke-width", "1.4");
  svg.setAttribute("stroke-linecap", "round");
  svg.setAttribute("stroke-linejoin", "round");
  const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
  path.setAttribute("d", ICONS[name] ?? ICONS.folder);
  svg.append(path);
  return svg;
}
const viewport = el("viewport");
const spacer = el("spacer");
const content = el("content");
const statusBar = el("status");
const syncProgress = el("sync-progress");
const reading = el("reading");
const emptyPane = el("empty");
const searchBox = el("search");
const toast = el("toast");
const sidebar = {
  accounts: el("accounts"),
  postboxesHeading: el("postboxes-heading"),
  postboxes: el("postboxes"),
  folders: el("folders"),
  categoriesHeading: el("categories-heading"),
  categories: el("categories"),
};
const scopeBar = el("scope");

const state = {
  accounts: [],
  account: null,
  // A postal address shown as an inbox in place of the account, or null.
  postbox: null,
  postboxes: [],
  // Letters per postbox, as Paperless last said, and how many are unread here.
  postboxTotals: new Map(),
  postboxUnread: new Map(),
  email: null,
  syncing: false,
  archive: null,
  trash: null,
  folders: [],
  filter: { ...NO_FILTER },
  // "mail", or one of the other ways of looking at it: "urgent" (Needs
  // attention) or "people". See people.js.
  view: "mail",
  // Smart mailboxes on the current account, as the sidebar last drew them.
  smartMailboxes: [],
  total: 0,
  // Sparse: offset -> row. Only what has been fetched.
  rows: new Map(),
  // Page offsets already requested, so a slow fetch is not asked for twice.
  requested: new Set(),
  selected: -1,
  // Rows picked out beside the cursor, by index, for acting on several at
  // once. Empty means "just the one under the cursor".
  selection: new Set(),
  // Where a shift-range starts.
  anchor: -1,
  searching: false,
  pool: [],
  firstRendered: -1,
};

/// Which date post is listed and sorted by: the date on the letter, or when it
/// was scanned. Kept across restarts — it is how someone reads their post.
let postOrder = (() => {
  try {
    return localStorage.getItem("postOrder") === "added" ? "added" : "created";
  } catch {
    return "created";
  }
})();

function setPostOrder(value) {
  postOrder = value;
  try {
    localStorage.setItem("postOrder", value);
  } catch {
    // A preference: without storage it lasts until the window closes.
  }
}

/// Which way mail is sorted by date. Newest first is what a mailbox is for;
/// oldest first is for working through a folder from the start.
let mailOrder = (() => {
  try {
    return localStorage.getItem("mailOrder") === "oldest" ? "oldest" : "newest";
  } catch {
    return "newest";
  }
})();

function setMailOrder(value) {
  mailOrder = value;
  try {
    localStorage.setItem("mailOrder", value);
  } catch {
    // As above: it lasts until the window closes.
  }
}

// -- data ------------------------------------------------------------------

/// Which list the pages being fetched are for. A reload starts a new one, and
/// a page that answers an older list — a folder or a view left while it was on
/// its way — is dropped rather than drawn into the new one.
let listGeneration = 0;

async function loadPage(offset) {
  if (state.requested.has(offset)) return;
  // A resize or scroll before start has picked an account draws the empty
  // list, and the empty list asks for its first page. There is nothing to ask
  // for yet; choosing the account reloads.
  if (!state.postbox && state.view === "mail" && state.account === null) return;
  state.requested.add(offset);
  const generation = listGeneration;

  try {
    // A postbox's page comes from Paperless, in the shape a page of mail has,
    // so everything below draws it without knowing which it is.
    const page = state.postbox
      ? await invoke("paper_documents", {
          id: state.postbox.id,
          offset,
          limit: PAGE,
          query: null,
          order: postOrder,
          category: state.filter.category,
        })
      : state.view !== "mail"
        ? await viewPage(offset, PAGE)
        : await invoke("messages", {
          account: state.account,
          offset,
          limit: PAGE,
          filter: {
            category: state.filter.category,
            unreadOnly: state.filter.unreadOnly,
            folder: state.filter.folder,
            oldestFirst: mailOrder === "oldest",
            smart: state.filter.smart,
          },
        });

    if (generation !== listGeneration) return;
    // Post arrived, so whatever the token was refused for before is over.
    if (state.postbox) paperRejected.delete(state.postbox.id);
    // The total can move under us while a sync is running, so it is taken
    // from every page rather than once at the start.
    if (page.total !== state.total) {
      state.total = page.total;
      spacer.style.height = `${state.total * ROW_HEIGHT}px`;
    }
    if (state.postbox) state.postboxTotals.set(state.postbox.id, page.total);
    page.rows.forEach((row, i) => state.rows.set(page.offset + i, row));
    render(true);
  } catch (err) {
    if (generation !== listGeneration) return;
    state.requested.delete(offset);
    const what = state.postbox ? `post from ${state.postbox.label}` : "messages";
    const token = state.postbox && /token/i.test(String(err));
    say(
      token
        ? `${state.postbox.label}: Paperless rejected kuverta's token — sign in again in settings (,)`
        : `could not load ${what}: ${err}`,
      true,
    );
    if (state.postbox) {
      if (token) paperRejected.add(state.postbox.id);
      else paperRejected.delete(state.postbox.id);
      checkPaperless();
    }
  }
}

/// Throws away everything fetched and asks again.
///
/// `keepPosition` is what makes triage bearable: archive the message under the
/// cursor and the next one slides into its place, rather than the list jumping
/// back to the top and making you find your way again. A filter change is the
/// other case — there the old position means nothing, so it resets.
async function reload({ keepPosition = false } = {}) {
  const wasAt = state.selected;
  listGeneration += 1;
  state.rows.clear();
  state.requested.clear();
  state.firstRendered = -1;
  state.searching = false;
  reading.hidden = true;
  emptyPane.hidden = false;
  if (!keepPosition) {
    state.selected = -1;
    viewport.scrollTop = 0;
  }

  await loadPage(0);
  await refreshSidebar();
  refreshCleanupCount();

  if (state.total === 0) {
    state.selected = -1;
  } else if (keepPosition) {
    // The row that was under the cursor has gone, so the same index is now
    // the one that followed it.
    select(Math.min(wasAt, state.total - 1));
  } else {
    select(0);
  }
  renderScope();
}

// -- sidebar ---------------------------------------------------------------

/** One row in the sidebar. The same shape for accounts, folders and categories. */
function navItem({ label, count, unread, active, className, onClick, title, icon }) {
  const button = document.createElement("button");
  button.className = `nav-item${className ? ` ${className}` : ""}${active ? " active" : ""}`;
  // The badge shows one number, and its weight is the only thing saying which
  // — too subtle to rely on alone, so the full answer is a hover away.
  if (title) button.title = title;
  if (icon) button.append(iconSvg(icon));

  const text = document.createElement("span");
  text.className = "label";
  text.textContent = label;
  button.append(text);

  // Unread is the number worth seeing. The total only shows when there is no
  // unread count to compete with it, so the column stays scannable.
  if (unread) {
    const badge = document.createElement("span");
    badge.className = "count unread";
    badge.textContent = unread;
    button.append(badge);
  } else if (count !== null && count !== undefined) {
    const badge = document.createElement("span");
    badge.className = "count";
    badge.textContent = count;
    button.append(badge);
  }

  if (onClick) button.onclick = onClick;
  return button;
}

async function refreshSidebar() {
  // Accounts. Hidden when there is only one, because a list of one is a label
  // pretending to be a choice — unless there is post beside it, when the
  // account is the way back to mail.
  renderProfileBar();
  sidebar.accounts.textContent = "";
  if (visibleAccounts().length > 1 || visiblePostboxes().length) {
    for (const account of visibleAccounts()) {
      sidebar.accounts.append(
        navItem({
          label: account.email,
          className: "account",
          active: !state.postbox && account.id === state.account,
          onClick: () => selectAccount(account),
        }),
      );
    }
  }

  // Each postal address is an inbox of its own: its post, beside the mail.
  sidebar.postboxes.textContent = "";
  sidebar.postboxesHeading.hidden = visiblePostboxes().length === 0;
  for (const postbox of visiblePostboxes()) {
    sidebar.postboxes.append(
      navItem({
        label: postbox.label || postbox.base_url,
        icon: "post",
        className: "account",
        count: state.postboxTotals.get(postbox.id) ?? null,
        unread: state.postboxUnread.get(postbox.id) || 0,
        active: state.postbox?.id === postbox.id,
        // A token-less address is configured and unreadable, which is worth
        // seeing without opening it.
        title: postbox.has_token
          ? `${postbox.label} — post in Paperless at ${postbox.base_url}`
          : `${postbox.label} — no Paperless token stored (settings)`,
        onClick: () => selectPostbox(postbox),
      }),
    );
  }
  renderPaperStatus();

  // Folders belong to a mail account; Paperless keeps post under its own tags.
  // Categories are for both: post is sorted by the same rules as mail.
  const onPost = state.postbox !== null;
  sidebar.folders.hidden = onPost;
  el("folders-heading").hidden = onPost;
  el("smart-heading").hidden = onPost;
  el("smart").hidden = onPost;
  sidebar.categories.hidden = false;
  sidebar.categoriesHeading.hidden = false;
  if (onPost) {
    const box = state.postbox;
    const counts = await invoke("paper_category_counts", { id: box.id }).catch(() => []);
    if (state.postbox === box) renderCategories(counts);
    return;
  }

  await renderAttentionNav();
  state.folders = await invoke("folders", { account: state.account });
  sidebar.folders.textContent = "";
  sidebar.folders.append(
    navItem({
      label: "All mail",
      icon: "all",
      count: null,
      active: state.view === "mail" && state.filter.folder === null && state.filter.smart === null,
      onClick: async () => {
        state.view = "mail";
        state.filter = { ...state.filter, folder: null, smart: null };
        await reload();
      },
    }),
  );
  for (const folder of state.folders) {
    const item = navItem({
      label: folder.label,
      icon: iconFor(folder),
      count: folder.total,
      unread: folder.unread,
      title: `${folder.name} — ${folder.unread} unread of ${folder.total}`,
      active: state.view === "mail" && state.filter.folder === folder.id && state.filter.smart === null,
      onClick: async () => {
        state.view = "mail";
        const next = state.filter.folder === folder.id && state.filter.smart === null ? null : folder.id;
        state.filter = { ...state.filter, folder: next, smart: null };
        await reload();
      },
    });
    // Its own menu: a mailbox inside it, or deleting it. See folders.js.
    item.addEventListener("contextmenu", (event) => {
      event.preventDefault();
      showFolderMenu(event, folder);
    });
    sidebar.folders.append(item);
  }

  // Smart mailboxes and mail waiting to go, each drawn by its own file.
  await renderSmartMailboxes();
  await renderOutboxNav();

  renderCategories(
    await invoke("category_counts", {
      account: state.account,
      folder: state.filter.folder,
    }),
  );
}

/// What the categories are counted over, so a count that changed with the
/// folder is not read as mail going missing.
function categoriesHeading() {
  if (state.postbox) return "Categories";
  const smart = state.smartMailboxes.find((m) => m.id === state.filter.smart);
  if (smart) return `Categories in ${smart.name}`;
  const folder = state.folders.find((f) => f.id === state.filter.folder);
  return folder ? `Categories in ${folder.label}` : "Categories";
}

/// The categories, with counts, as filters — for mail or for post.
function renderCategories(counts) {
  sidebar.categoriesHeading.textContent = categoriesHeading();
  sidebar.categories.textContent = "";
  // Unread-only filters mail in the store. Post's read state is kuverta's own
  // and no filter Paperless can apply, so a postbox shows its unread count on
  // the postbox instead.
  if (!state.postbox) {
    sidebar.categories.append(
      navItem({
        label: "Unread only",
        icon: "inbox",
        active: state.filter.unreadOnly,
        onClick: async () => {
          state.view = "mail";
          state.filter = { ...state.filter, unreadOnly: !state.filter.unreadOnly };
          await reload();
        },
      }),
    );
  }
  for (const [category, count] of counts) {
    sidebar.categories.append(
      navItem({
        label: category,
        icon: "tag",
        count,
        active: state.filter.category === category,
        onClick: async () => {
          if (!state.postbox) state.view = "mail";
          const next = state.filter.category === category ? null : category;
          state.filter = { ...state.filter, category: next };
          await reload();
        },
      }),
    );
  }
}

/// Says what the list is showing.
///
/// A filtered list that does not announce itself looks exactly like mail going
/// missing, so the narrowing is always on screen with a way out of it.
function renderScope() {
  scopeBar.textContent = "";
  const parts = [];
  if (state.searching) parts.push("search results");
  if (state.filter.smart !== null) {
    const smart = state.smartMailboxes.find((m) => m.id === state.filter.smart);
    if (smart) parts.push(smart.name);
  }
  if (state.filter.folder !== null) {
    const folder = state.folders.find((f) => f.id === state.filter.folder);
    if (folder) parts.push(folder.label);
  }
  if (state.filter.category) parts.push(state.filter.category);
  if (state.filter.unreadOnly) parts.push("unread");

  const picked = state.selection.size;
  if (picked > 1) {
    const many = document.createElement("span");
    many.textContent = `${picked} picked`;
    many.className = "picked-count";
    scopeBar.append(many);
    const drop = document.createElement("button");
    drop.textContent = "unpick";
    drop.onclick = clearSelection;
    scopeBar.append(drop);
  }

  const label = document.createElement("span");
  const letters = `letter${state.total === 1 ? "" : "s"}`;
  if (!state.postbox && state.view === "urgent") {
    label.textContent = `${state.total} need${state.total === 1 ? "s" : ""} attention`;
    label.className = "scope-name";
    scopeBar.append(label, ...viewTools());
    return;
  }
  if (!state.postbox && state.view === "people") {
    label.textContent = `${state.total} ${state.total === 1 ? "person" : "people"}`;
    scopeBar.append(label, ...viewTools(), viewToggle());
    return;
  }
  label.textContent = parts.length
    ? `${state.total} in ${parts.join(" · ")}`
    : state.postbox
      ? `${state.total} ${letters} to ${state.postbox.label}`
      : `${state.total} messages`;
  scopeBar.append(label);

  if (parts.length) {
    const clear = document.createElement("button");
    clear.textContent = "clear";
    clear.onclick = async () => {
      searchBox.value = "";
      state.filter = { ...NO_FILTER };
      await reload();
    };
    scopeBar.append(clear);
  }

  // Mail is sorted by date; which end it starts at is the choice.
  if (!state.postbox) {
    scopeBar.append(viewToggle());
    const toggle = document.createElement("span");
    toggle.className = "order";
    for (const [value, text, title] of [
      ["newest", "newest", "Newest mail at the top"],
      ["oldest", "oldest", "Oldest mail at the top"],
    ]) {
      const button = document.createElement("button");
      button.textContent = text;
      button.title = title;
      if (mailOrder === value) button.className = "active";
      button.onclick = async () => {
        if (mailOrder === value) return;
        setMailOrder(value);
        await reload();
      };
      toggle.append(button);
    }
    scopeBar.append(toggle);
  }

  // Post has two dates worth sorting by: when the letter was written, and when
  // it came through the letterbox and was scanned.
  if (state.postbox) {
    const toggle = document.createElement("span");
    toggle.className = "order";
    for (const [value, text, title] of [
      ["created", "letter date", "Sort and date by the date on the letter"],
      ["added", "scanned", "Sort and date by when it was scanned"],
    ]) {
      const button = document.createElement("button");
      button.textContent = text;
      button.title = title;
      if (postOrder === value) button.className = "active";
      button.onclick = async () => {
        if (postOrder === value) return;
        setPostOrder(value);
        await reload();
      };
      toggle.append(button);
    }
    scopeBar.append(toggle);
  }
}

/// Shows a postal address's post as an inbox: what Paperless holds for it,
/// newest first, read like mail.
async function selectPostbox(postbox) {
  state.postbox = postbox;
  state.view = "mail";
  hideConversation();
  statusBar.textContent = postbox.label;
  state.filter = { ...NO_FILTER };
  closeCompose();
  if (!postbox.has_token) {
    say(`${postbox.label} has no Paperless token stored — add it in settings (,)`, true);
  }
  await reload();
  autoTranscribe();
}

/// Moving, deleting, marking read, undo and replying: things mail has and post
/// does not. Paperless owns a letter and this only reads it, so in a postbox
/// these keys say so rather than doing something to the account behind it.
function mailOnly(action) {
  return () =>
    state.postbox
      ? say(`${state.postbox.label} is post — that is done in Paperless`, true)
      : state.view === "people" && action !== openCompose
        ? say("People lists people, not messages — open one, or switch to Messages", true)
        : action();
}

// -- whether Paperless is up -------------------------------------------------

/// Each postbox's Paperless as last asked: answering, and whether kuverta
/// installed it and so can start it. Filled by `checkPaperless`.
const paperHealth = new Map();
/// Postboxes whose Paperless answered but refused kuverta's token. A token
/// can be revoked in Paperless, or belong to an instance that was rebuilt;
/// either way it is signing in again that fixes it, not waiting.
const paperRejected = new Set();
/// What starting it is doing, while it is; null otherwise.
let paperStarting = null;

/// Asks each postbox's Paperless whether it is there, and redraws the line
/// under Post. In the background: a Paperless that is away answers slowly.
async function checkPaperless() {
  const health = await invoke("paperless_health").catch(() => null);
  if (!health) return;
  paperHealth.clear();
  for (const h of health) paperHealth.set(h.id, h);
  renderPaperStatus();
}

function renderPaperStatus() {
  const line = el("paper-status");
  const shown = visiblePostboxes()
    .map((p) => paperHealth.get(p.id))
    .filter(Boolean);
  line.hidden = !shown.length && paperStarting === null;
  if (line.hidden) return;

  const down = shown.filter((h) => !h.answering);
  const rejected = shown.filter((h) => h.answering && paperRejected.has(h.id));
  const dot = document.createElement("span");
  dot.className = "dot";
  const text = document.createElement("span");
  text.className = "grow";
  line.replaceChildren(dot, text);

  if (paperStarting !== null) {
    line.className = "paper-status";
    text.textContent = paperStarting;
    text.title = paperStarting;
    return;
  }
  line.className = `paper-status ${down.length || rejected.length ? "down" : "up"}`;
  if (!down.length && rejected.length) {
    // Running, and refusing us: a wait will not help, so say what will.
    text.textContent = "Paperless rejected kuverta's token";
    text.title = `Sign in again for ${rejected.map((h) => h.base_url).join(", ")}`;
    const again = document.createElement("button");
    again.type = "button";
    again.textContent = "Sign in again";
    again.onclick = () => openPostboxSettings(rejected[0].id);
    line.append(again);
    return;
  }
  if (!down.length) {
    text.textContent = "Paperless is running";
    text.title = shown.map((h) => h.base_url).join(", ");
    return;
  }
  text.textContent = "Paperless is not running";
  text.title = `Nothing answers at ${down.map((h) => h.base_url).join(", ")}`;
  if (down.some((h) => h.startable)) {
    const start = document.createElement("button");
    start.type = "button";
    start.textContent = "Start";
    start.title = "Start Docker if need be, then Paperless";
    start.onclick = startPaperless;
    line.append(start);
  } else {
    text.title += " — it runs elsewhere, so start it there";
  }
}

/// Settings, open on one postal address: where its token is signed for again.
async function openPostboxSettings(id) {
  await openSettings();
  const address = paper.addresses.find((a) => a.id === id);
  if (address) fillPaper(address);
}

async function startPaperless() {
  if (paperStarting !== null) return;
  paperStarting = "starting…";
  renderPaperStatus();
  const channel = new window.__TAURI__.core.Channel();
  channel.onmessage = (line) => {
    paperStarting = line;
    renderPaperStatus();
  };
  try {
    await invoke("start_paperless", { onOutput: channel });
    say("Paperless is running");
  } catch (err) {
    say(`could not start Paperless: ${err}`, true);
  } finally {
    paperStarting = null;
    await checkPaperless();
  }
  countPostboxes();
  if (state.postbox) await reload();
}

/// How many letters each postbox holds, asked in the background: the sidebar
/// shows it when it arrives, and a slow or absent Paperless holds nothing up.
function countPostboxes() {
  checkPaperless();
  const redraw = () => (state.account !== null || state.postbox ? refreshSidebar() : null);
  for (const postbox of state.postboxes) {
    if (!postbox.has_token) continue;
    invoke("paper_documents", { id: postbox.id, offset: 0, limit: 1, query: null })
      .then((page) => {
        state.postboxTotals.set(postbox.id, page.total);
        return redraw();
      })
      .catch(() => {});
    invoke("paper_unread_count", { id: postbox.id })
      .then((unread) => {
        state.postboxUnread.set(postbox.id, unread);
        return redraw();
      })
      .catch(() => {});
  }
}

/// Post's read state is kuverta's own: opening a letter reads it, u toggles.
async function markPostRead(row, read) {
  const box = state.postbox;
  if (!box) return;
  try {
    await invoke("set_paper_read", { id: box.id, documentId: row.id, read });
    row.unread = !read;
    render(true);
    state.postboxUnread.set(box.id, await invoke("paper_unread_count", { id: box.id }));
    await refreshSidebar();
  } catch (err) {
    say(`could not mark it ${read ? "read" : "unread"}: ${err}`, true);
  }
}

// -- reading scans with the vision model ---------------------------------------

/// Letters being read now, by postbox and document. One at a time: a vision
/// model reading two pages at once on a laptop reads both slowly.
const transcribing = new Set();
const transcribeQueue = [];
let transcribeRunning = false;
/// Why the vision model could not be used, once it has failed — so new post
/// stops asking, and the reading pane can say why.
let visionFailed = null;

function transcribePost(box, documentId, { force = false } = {}) {
  const key = `${box.id}:${documentId}`;
  if (transcribing.has(key) || transcribeQueue.some((job) => job.key === key)) return;
  if (visionFailed && !force) return;
  if (force) visionFailed = null;
  transcribeQueue.push({ key, box, documentId });
  runTranscriptions();
}

async function runTranscriptions() {
  if (transcribeRunning) return;
  transcribeRunning = true;
  while (transcribeQueue.length) {
    const job = transcribeQueue.shift();
    transcribing.add(job.key);
    refreshOpenLetter(job.key);
    try {
      await invoke("paper_transcribe", { id: job.box.id, documentId: job.documentId });
      transcribing.delete(job.key);
      // The row's sender, subject and category may all change with readable
      // text, so the list is asked again; the letter being read stays open.
      if (state.postbox?.id === job.box.id) await refreshPostbox();
    } catch (err) {
      transcribing.delete(job.key);
      visionFailed = String(err);
      transcribeQueue.length = 0;
      say(`the vision model could not read the scan: ${err}`, true);
      refreshOpenLetter(job.key);
    }
  }
  transcribeRunning = false;
}

/// New post is read as it arrives, so the list says who it is from and what it
/// is about — and sorts it — from text a person could read. Only letters
/// nobody has opened yet.
function autoTranscribe() {
  if (!state.postbox || visionFailed) return;
  for (const row of state.rows.values()) {
    if (row.unread && !row.transcribed_by) transcribePost(state.postbox, row.id);
  }
}

/// Redraws the open letter if it is the one `key` names.
function refreshOpenLetter(key) {
  const row = state.rows.get(state.selected);
  if (reading.hidden || !state.postbox || !row || `${state.postbox.id}:${row.id}` !== key) return;
  openSelected();
}

function showTranscriptNote(detail, row) {
  const note = el("reading-note");
  const button = el("reading-transcribe");
  const key = `${state.postbox.id}:${row.id}`;
  button.onclick = () => transcribePost(state.postbox, row.id, { force: true });
  if (transcribing.has(key) || transcribeQueue.some((job) => job.key === key)) {
    note.textContent = "Reading the scan with the vision model… it takes a little while a page.";
    button.hidden = true;
  } else if (detail.transcript_model) {
    note.textContent = `Read from the scan by ${detail.transcript_model}. Where the scan is unclear it can get a word wrong — the PDF tab has the page itself.`;
    button.textContent = "Read again";
    button.hidden = false;
  } else {
    note.textContent = visionFailed
      ? `This is Paperless's OCR. The vision model could not read the scan: ${visionFailed}`
      : "This is Paperless's OCR.";
    button.textContent = "Read with the vision model";
    button.hidden = false;
  }
}

// -- post arriving -------------------------------------------------------------

/// Post has no push: Paperless tells nobody when a letter arrives. So the window
/// asks — the open postbox every so often, every postbox's count less often —
/// and a scanned letter appears on its own, the way mail does after a sync.
/// A test page may shorten the wait.
const POST_POLL_MS = window.__kuvertaPostPollMs ?? 20000;
const COUNT_EVERY = 3; // polls between count refreshes: a minute at the default
let postPolls = 0;

async function checkForPost() {
  postPolls += 1;
  // Hidden, there is no one to show it to.
  if (document.hidden) return;
  if (postPolls % COUNT_EVERY === 0) countPostboxes();

  const box = state.postbox;
  if (!box || state.searching || !box.has_token) return;
  try {
    const newest = await invoke("paper_documents", {
      id: box.id,
      offset: 0,
      limit: 1,
      query: null,
      order: postOrder,
      category: state.filter.category,
    });
    if (state.postbox !== box) return; // moved elsewhere while asking
    const shownNewest = state.rows.get(0)?.id ?? null;
    const topId = newest.rows[0]?.id ?? null;
    if (newest.total === state.total && topId === shownNewest) return;

    const arrived = newest.total - state.total;
    await refreshPostbox();
    autoTranscribe();
    countPostboxes();
    if (arrived > 0) say(`${arrived} new letter${arrived === 1 ? "" : "s"} to ${box.label}`);
  } catch {
    // Paperless away or the Mac asleep: ask again next time, quietly.
  }
}

/// Reloads the open postbox without losing the place: the letter that was
/// selected stays selected — new post lands above it and moves it down — and
/// stays open if it was being read.
async function refreshPostbox() {
  const readingOpen = !reading.hidden;
  const selectedId = state.rows.get(state.selected)?.id ?? null;
  const scrolled = viewport.scrollTop;

  await reload();

  if (selectedId === null) return;
  for (const [index, row] of state.rows) {
    if (row.id !== selectedId) continue;
    viewport.scrollTop = scrolled;
    select(index);
    if (readingOpen) await openSelected();
    return;
  }
}

setInterval(checkForPost, POST_POLL_MS);

async function selectAccount(account) {
  state.postbox = null;
  if (assistantOpen()) queueMicrotask(() => {
    ensureConversation();
    if (currentTab() === "tasks") refreshTasks();
  });
  state.view = "mail";
  hideConversation();
  state.account = account.id;
  state.email = account.email;
  statusBar.textContent = account.email;
  state.filter = { ...NO_FILTER };

  const special = await invoke("special_folders", { account: state.account });
  state.archive = special.archive;
  state.trash = special.trash;

  await reload();
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

    // Line one: sender, any tag, then the date pushed to the right.
    const top = document.createElement("div");
    top.className = "top";
    const sender = document.createElement("span");
    sender.className = "sender";
    const tag = document.createElement("span");
    tag.className = "tag";
    const date = document.createElement("span");
    date.className = "date";
    top.append(sender, tag, date);

    const subject = document.createElement("div");
    subject.className = "subject";
    const snippet = document.createElement("div");
    snippet.className = "snippet";

    row.append(unread, top, subject, snippet);
    content.append(row);
    state.pool.push({ row, unread, top, date, sender, subject, snippet, tag });
  }
  state.firstRendered = -1;
}

const timeOnly = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" });
const weekday = new Intl.DateTimeFormat(undefined, { weekday: "short" });
const dayMonth = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short" });
const withYear = new Intl.DateTimeFormat(undefined, {
  day: "numeric",
  month: "short",
  year: "numeric",
});

/// Time today, weekday this week, date beyond — the resolution you actually
/// want at each distance, and short enough not to crowd the sender.
function listDate(seconds) {
  if (!seconds) return "";
  const when = new Date(seconds * 1000);
  const now = new Date();
  const sameDay = when.toDateString() === now.toDateString();
  if (sameDay) return timeOnly.format(when);

  const days = (now - when) / 86400000;
  if (days < 7 && days >= 0) return weekday.format(when);
  if (when.getFullYear() === now.getFullYear()) return dayMonth.format(when);
  return withYear.format(when);
}

const fullDate = new Intl.DateTimeFormat(undefined, {
  weekday: "short",
  day: "numeric",
  month: "short",
  year: "numeric",
  hour: "2-digit",
  minute: "2-digit",
});

function formatDate(seconds) {
  if (!seconds) return "";
  return fullDate.format(new Date(seconds * 1000));
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
    node.row.classList.toggle("picked", state.selection.has(index));

    if (!row) {
      // The page is still in flight. A placeholder keeps the row height
      // correct so the scrollbar does not jump when it lands.
      node.row.classList.add("pending");
      node.unread.className = "dot";
      node.date.textContent = "";
      node.sender.textContent = "";
      node.subject.textContent = "…";
      node.snippet.textContent = "";
      node.tag.textContent = "";
      continue;
    }

    node.row.classList.remove("pending");
    node.unread.className = row.unread ? "dot on" : "dot";
    const scanned = state.postbox && postOrder === "added";
    node.date.textContent = listDate(scanned ? (row.added_utc ?? row.date_utc) : row.date_utc);
    node.sender.textContent = row.from;
    node.subject.textContent = row.subject;
    node.snippet.textContent = row.snippet ?? "";
    node.tag.textContent = [row.has_attachments ? "📎" : "", row.category]
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
  if (!state.postbox && state.view === "people") {
    await openConversation(row);
    return;
  }
  hideConversation();

  try {
    if (state.postbox) {
      const detail = await invoke("paper_document", { id: state.postbox.id, documentId: row.id });
      const pages = detail.row.page_count;
      reading.hidden = false;
      emptyPane.hidden = true;
      el("reading-actions").hidden = true;
      el("reading-urgency").hidden = true;
      showSecurity(null);
      hideAttachments();
      el("reading-subject").textContent = detail.row.subject || "(untitled)";
      el("reading-meta").textContent = [
        detail.row.from,
        formatDate(detail.row.date_utc),
        state.postbox.label,
        pages ? `${pages} page${pages === 1 ? "" : "s"}` : "",
        `Paperless document ${detail.row.id}`,
      ]
        .filter(Boolean)
        .join("  ·  ");
      // The OCR text is the body. It is missing while Paperless is still
      // reading a scan, which is not the same as a letter with nothing on it.
      el("reading-body").textContent =
        detail.body_text ?? "(no text yet — Paperless may still be reading this scan)";
      if (scan.key !== scanKey(row)) clearScan();
      showReadingView();
      showTranscriptNote(detail, row);
      if (row.unread) markPostRead(row, true);
      if (!detail.transcript_model && !visionFailed) {
        transcribePost(state.postbox, row.id);
        showTranscriptNote(detail, row);
      }
      return;
    }

    clearScan();
    showReadingView();
    const detail = await invoke("message", { account: state.account, id: row.id });
    reading.hidden = false;
    emptyPane.hidden = true;
    el("reading-actions").hidden = false;
    showSecurity(detail);
    showUrgency(row.id);
    el("reading-subject").textContent = detail.subject ?? "(no subject)";
    el("reading-meta").textContent = [
      detail.from,
      formatDate(detail.date_utc),
      detail.folders.join(", "),
    ]
      .filter(Boolean)
      .join("  ·  ");
    // HTML mail arrives already rendered to text, so an empty body really
    // means an empty body — an attachment-only message, or one whose stored
    // copy has gone.
    el("reading-body").textContent = detail.body_text ?? "(no readable body)";
    showAttachments(detail);
  } catch (err) {
    say(`could not open: ${err}`, true);
  }
}

// -- post: the text, or the scan itself ----------------------------------------

/// Which view a letter opens on. Kept across letters and restarts: someone
/// whose scans read badly wants the page every time, not a click per letter.
let readingView = (() => {
  try {
    return localStorage.getItem("readingView") === "pdf" ? "pdf" : "text";
  } catch {
    return "text";
  }
})();

/// The scan on screen, by postbox and document: ids are Paperless's, so two
/// postboxes on two instances can share one.
const scan = { key: null, url: null, loading: null };
const scanKey = (row) => (state.postbox ? `${state.postbox.id}:${row.id}` : null);

function showReadingView() {
  const onPost = state.postbox !== null;
  const pdf = onPost && readingView === "pdf";
  el("reading-tabs").hidden = !onPost;
  el("reading-post").hidden = !onPost || pdf;
  el("reading-body").hidden = pdf;
  el("reading-file").hidden = !pdf;
  for (const tab of el("reading-tabs").querySelectorAll("[data-view]")) {
    const active = tab.dataset.view === (pdf ? "pdf" : "text");
    tab.classList.toggle("active", active);
    tab.setAttribute("aria-selected", String(active));
  }
  if (pdf) loadScan();
}

function setReadingView(view) {
  readingView = view;
  try {
    localStorage.setItem("readingView", view);
  } catch {
    // A preference, not state: without storage it lasts until the window closes.
  }
  if (!reading.hidden) showReadingView();
}

/// Fetches the letter's file through the app — the window may not reach
/// Paperless itself, and the download needs the token — and shows it.
async function loadScan() {
  const row = state.rows.get(state.selected);
  if (!row || !state.postbox) return;
  const key = scanKey(row);
  if (scan.key === key && (scan.url || scan.loading)) return;

  clearScan();
  scan.key = key;
  el("reading-file-status").textContent = "Loading the scan from Paperless…";
  const loading = invoke("paper_file", { id: state.postbox.id, documentId: row.id });
  scan.loading = loading;
  try {
    const bytes = new Uint8Array(await loading);
    if (scan.key !== key) return; // moved on to another letter meanwhile
    const type = sniff(bytes);
    scan.url = URL.createObjectURL(new Blob([bytes], { type }));
    const frame = el("reading-pdf");
    const image = el("reading-image");
    if (type === "application/pdf") {
      frame.src = scan.url;
      frame.hidden = false;
    } else {
      image.src = scan.url;
      image.hidden = false;
    }
    el("reading-file-status").textContent = "";
  } catch (err) {
    if (scan.key === key) el("reading-file-status").textContent = `could not load the scan: ${err}`;
  } finally {
    if (scan.loading === loading) scan.loading = null;
  }
}

function clearScan() {
  if (scan.url) URL.revokeObjectURL(scan.url);
  Object.assign(scan, { key: null, url: null, loading: null });
  for (const id of ["reading-pdf", "reading-image"]) {
    el(id).removeAttribute("src");
    el(id).hidden = true;
  }
  el("reading-file-status").textContent = "";
}

/// What the bytes are. The app hands over only bytes, and Paperless's download
/// is a PDF when it archived the scan and the original — often a JPEG — when not.
function sniff(bytes) {
  const starts = (...prefix) => prefix.every((value, i) => bytes[i] === value);
  if (starts(0x25, 0x50, 0x44, 0x46)) return "application/pdf";
  if (starts(0xff, 0xd8)) return "image/jpeg";
  if (starts(0x89, 0x50, 0x4e, 0x47)) return "image/png";
  return "application/octet-stream";
}

for (const tab of el("reading-tabs").querySelectorAll("[data-view]")) {
  tab.onclick = () => setReadingView(tab.dataset.view);
}

/// Which rows an action applies to: the picked ones, or the one under the
/// cursor when nothing is picked.
function actingOn() {
  const picked = [...state.selection].filter((index) => index < state.total).sort((a, b) => a - b);
  return picked.length ? picked : state.selected >= 0 ? [state.selected] : [];
}

function clearSelection() {
  if (!state.selection.size) return;
  state.selection.clear();
  state.anchor = -1;
  render(true);
}

/// Everything from the anchor to `index`, which is what shift means in every
/// list: the block between where you started and where you are now.
function selectRange(index) {
  if (state.total === 0) return;
  index = Math.max(0, Math.min(index, state.total - 1));
  if (state.anchor < 0) state.anchor = state.selected >= 0 ? state.selected : index;
  const [from, to] = state.anchor <= index ? [state.anchor, index] : [index, state.anchor];
  state.selection = new Set();
  for (let at = from; at <= to; at++) state.selection.add(at);
  state.selected = index;
  keepInView();
  render(true);
  renderScope();
}

/// Adds or removes one row, for picking out several that are not together.
function toggleSelected(index) {
  if (state.total === 0) return;
  if (state.selection.has(index)) state.selection.delete(index);
  else state.selection.add(index);
  state.selected = index;
  state.anchor = index;
  keepInView();
  render(true);
  renderScope();
}

function keepInView() {
  const top = state.selected * ROW_HEIGHT;
  const bottom = top + ROW_HEIGHT;
  if (top < viewport.scrollTop) viewport.scrollTop = top;
  else if (bottom > viewport.scrollTop + viewport.clientHeight) {
    viewport.scrollTop = bottom - viewport.clientHeight;
  }
}

function select(index) {
  if (state.total === 0) return;
  state.selection.clear();
  state.anchor = -1;
  state.selected = Math.max(0, Math.min(index, state.total - 1));

  // Keep the selection in view, which is what makes j/k usable at all.
  keepInView();
  render(true);
  renderScope();
  if (!reading.hidden || !conversationPane.hidden) openSelected();
}

// -- changes ---------------------------------------------------------------

let toastTimer = null;

function say(message, isError = false) {
  // An error shown is an error worth having in the log a bug report attaches.
  if (isError) invoke("log_ui", { level: "error", message: String(message) }).catch(() => {});
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
  const indexes = actingOn();
  const rows = indexes.map((index) => state.rows.get(index)).filter(Boolean);
  if (!rows.length) return 0;

  let done = 0;
  let failed = null;
  // One at a time: each is its own queued change with its own undo, and a
  // server that refuses the third should not lose the first two.
  for (const row of rows) {
    try {
      await invoke(command, { account: state.account, id: row.id, ...args });
      done += 1;
    } catch (err) {
      failed = err;
      break;
    }
  }
  state.selection.clear();
  state.anchor = -1;

  if (done) {
    const many = done > 1 ? ` ${done} messages` : "";
    say(failed ? `${describe}${many}, then: ${failed}` : `${describe}${many} — z to undo`, Boolean(failed));
  } else if (failed) {
    say(String(failed), true);
  }
  await reload({ keepPosition: true });
  return done;
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
  // Which messages went, read before they leave the list: one of them is
  // what the look-alikes are found from.
  const gone = actingOn()
    .map((index) => state.rows.get(index))
    .filter(Boolean);
  const moved = await act("move_to", { target: state.trash }, `moved to ${state.trash}`);
  // Only after a delete that happened: "more like the one you deleted" about
  // one that is still there would be a question about nothing.
  if (gone.length === 1 && moved === 1) offerSimilar(gone[0]);
}

async function toggleRead() {
  const row = state.rows.get(state.selected);
  if (!row) return;
  if (!state.postbox && state.view === "people") return;
  if (state.postbox) {
    await markPostRead(row, row.unread);
    say(row.unread ? "marked unread" : "marked read");
    return;
  }
  await act("set_read", { read: row.unread }, row.unread ? "marked read" : "marked unread");
}

async function undo() {
  try {
    const change = await invoke("undo", { account: state.account });
    say(change ? `undone: ${change.what}` : "nothing to undo");
    if (change) await reload({ keepPosition: true });
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
    sign: el("compose-sign").checked,
    encrypt: el("compose-encrypt").checked,
  };
}

function closeCompose() {
  compose.pane.hidden = true;
  el("schedule-menu").hidden = true;
  el("compose-sign").checked = false;
  el("compose-encrypt").checked = false;
  resetComposeSecurity();
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
  emptyPane.hidden = true;
  (compose.to.value ? compose.body : compose.to).focus();
  await refreshEnvelope();
}

/// Fills compose from a draft the core kept — a scheduled message being
/// edited again. Replies keep what they reply to, so threading survives.
async function openComposeWith(draft) {
  closeCompose();
  compose.replyTo = draft.reply_to ?? null;
  compose.replyAll = Boolean(draft.reply_all);
  compose.forward = draft.forward ?? null;
  compose.what.textContent = "Scheduled message";
  compose.to.value = (draft.to ?? []).join(", ");
  compose.cc.value = (draft.cc ?? []).join(", ");
  compose.bcc.value = (draft.bcc ?? []).join(", ");
  compose.subject.value = draft.subject ?? "";
  compose.body.value = draft.body ?? "";
  el("compose-sign").checked = Boolean(draft.sign);
  el("compose-encrypt").checked = Boolean(draft.encrypt);
  securityChosen = true;
  compose.pane.hidden = false;
  reading.hidden = true;
  emptyPane.hidden = true;
  compose.body.focus();
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
  // Which recipients have keys changes with the recipients. See encryption.js.
  await refreshComposeSecurity();
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
el("new-message").onclick = () => (state.postbox ? say("post cannot be answered from here", true) : openCompose());
el("open-settings").onclick = () => openSettings();

for (const button of el("reading-actions").querySelectorAll("[data-act]")) {
  button.onclick = () =>
    ({
      reply: () => openCompose({ replyAll: false }),
      "reply-all": () => openCompose({ replyAll: true }),
      forward: () => openCompose({ forward: true }),
      ask: () => askAboutMessages(actingOn().map((index) => state.rows.get(index)).filter(Boolean)),
      archive,
      trash,
    })[button.dataset.act]();
}

/// How much bulk mail is still in the Inbox, beside Cleanup in the header.
async function refreshCleanupCount() {
  const badge = el("cleanup-count");
  const link = el("cleanup-link");
  if (state.account === null) {
    badge.hidden = true;
    return;
  }
  try {
    const counts = await invoke("cleanup_counts", { account: state.account });
    badge.textContent = counts.bulk_in_inbox > 999 ? "999+" : String(counts.bulk_in_inbox);
    badge.hidden = counts.bulk_in_inbox === 0;
    link.title =
      `${counts.bulk_in_inbox} newsletter, marketing and notification message${counts.bulk_in_inbox === 1 ? "" : "s"} in the Inbox` +
      ` · ${counts.unsubscribable} sender${counts.unsubscribable === 1 ? "" : "s"} you can unsubscribe from`;
  } catch {
    badge.hidden = true;
  }
}
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
  if (state.postbox) {
    // Post is read from Paperless as it is shown, so syncing is asking again.
    await reload({ keepPosition: true });
    say(`${state.postbox.label}: ${state.total} letter${state.total === 1 ? "" : "s"} in Paperless`);
    return;
  }
  if (state.syncing) return;
  state.syncing = true;
  statusBar.textContent = `${state.email} — syncing…`;
  showSyncProgress(null);

  try {
    // The percentage counts folders done plus the share of the one in hand,
    // which is as close to honest as it gets before the sizes are in.
    const channel = new window.__TAURI__.core.Channel();
    channel.onmessage = (at) => {
      if (!state.syncing) return;
      showSyncProgress(at);
    };
    const s = await invoke("sync", { email: state.email, onProgress: channel });
    const parts = [];
    if (s.changes_sent) parts.push(`${s.changes_sent} change(s) sent`);
    if (s.changes_refused) parts.push(`${s.changes_refused} refused`);
    if (s.inserted) parts.push(`${s.inserted} new`);
    if (s.expunged) parts.push(`${s.expunged} gone`);
    say(parts.length ? parts.join(", ") : "nothing new");
    await reload({ keepPosition: true });
    // New mail gets its urgency judged and the tasks run, in the background.
    judgeUrgency();
    runTasks();
  } catch (err) {
    say(String(err), true);
  } finally {
    state.syncing = false;
    syncProgress.hidden = true;
    statusBar.textContent = state.email;
  }
}

/// Shows where the sync is, or hides the bar when passed nothing.
///
/// The folder is named rather than counted: "Archive" says more about a sync
/// sitting still than "folder 7 of 19" does.
function showSyncProgress(at) {
  if (!at || at.fraction === null || at.fraction === undefined) {
    syncProgress.hidden = true;
    return;
  }
  syncProgress.value = at.fraction;
  syncProgress.hidden = false;
  const percent = Math.round(at.fraction * 100);
  const within = at.messages_total
    ? ` — ${at.messages_done} of ${at.messages_total}`
    : "";
  statusBar.textContent = `${state.email} — syncing ${percent}%: ${at.folder}${within}`;
}

// -- search ----------------------------------------------------------------

async function runSearch(query) {
  if (!query.trim() || (!state.postbox && state.view === "people")) {
    // In People the box narrows the people, which the next page asks for.
    await reload();
    return;
  }
  try {
    // In a postbox, Paperless's own full-text search, over the scanned text.
    const rows = state.postbox
      ? (await invoke("paper_documents", { id: state.postbox.id, offset: 0, limit: 200, query })).rows
      : await invoke("search", {
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
    renderScope();
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
  // Shift with the same keys picks a block, as it does in a file list.
  J: () => selectRange(state.selected + 1),
  K: () => selectRange(state.selected - 1),
  x: () => toggleSelected(state.selected),
  Enter: openSelected,
  o: openSelected,
  e: mailOnly(archive),
  "#": mailOnly(trash),
  Delete: mailOnly(trash),
  Backspace: mailOnly(trash),
  u: toggleRead,
  z: mailOnly(undo),
  r: sync,
  v: () => {
    if (state.postbox) setReadingView(readingView === "pdf" ? "text" : "pdf");
  },
  c: mailOnly(() => openCompose()),
  ",": openSettings,
  i: () => setAssistantOpen(!assistantOpen()),
  I: mailOnly(() => askAboutMessages(actingOn().map((index) => state.rows.get(index)).filter(Boolean))),
  R: mailOnly(() => openCompose({ replyAll: false })),
  A: mailOnly(() => openCompose({ replyAll: true })),
  f: mailOnly(() => openCompose({ forward: true })),
};

// Going back to the list takes the keys back with it: while the chat input
// has focus every key belongs to it, and j, x or e would otherwise do nothing.
content.addEventListener("mousedown", () => {
  const focused = document.activeElement;
  if (focused && el("assistant").contains(focused) && focused.tagName === "TEXTAREA") focused.blur();
});

document.addEventListener("keydown", async (event) => {
  // The assistant is a form too, and Escape does not throw away what it has
  // been told.
  if (!el("setup").hidden) return;

  // A dialog on top owns the keys; Escape closes it.
  const dialog = [...document.querySelectorAll(".sheet.dialog")].find((sheet) => !sheet.hidden);
  if (dialog) {
    if (event.key === "Escape") closeDialog(dialog);
    return;
  }
  if (!el("context-menu").hidden && event.key === "Escape") {
    el("context-menu").hidden = true;
    return;
  }
  if (!el("schedule-menu").hidden && event.key === "Escape") {
    el("schedule-menu").hidden = true;
    return;
  }

  // The settings sheet is a form: every key belongs to whatever field has
  // focus, and none of them are triage shortcuts.
  if (!settings.sheet.hidden) {
    if (event.key === "Escape") await closeSettings();
    return;
  }

  // While composing, the keys belong to the fields — otherwise typing "e"
  // into a subject line would archive something.
  if (compose.pane.contains(event.target)) {
    // The send-later menu is inside compose and has its own keys: ⌘↩ there
    // must not also send the message now.
    if (el("schedule-menu").contains(event.target)) return;
    if (event.key === "Escape") closeCompose();
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) await sendDraft();
    return;
  }

  if (el("assistant").contains(event.target) && ["TEXTAREA", "INPUT", "SELECT"].includes(event.target.tagName)) {
    return;
  }

  if (conversationPane.contains(event.target) && event.target.tagName === "TEXTAREA") {
    if (event.key === "Escape") event.target.blur();
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
    clearSelection();
    reading.hidden = true;
    hideConversation();
    emptyPane.hidden = false;
    return;
  }

  // Shift with an arrow key reports the same key, so the block is taken here
  // rather than from the table.
  if (event.shiftKey && (event.key === "ArrowDown" || event.key === "ArrowUp")) {
    event.preventDefault();
    selectRange(state.selected + (event.key === "ArrowDown" ? 1 : -1));
    return;
  }

  // ⌘C copies, ⌘R is not sync: a single-letter shortcut is the letter alone.
  if (event.metaKey || event.ctrlKey || event.altKey) return;

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
  if (index < 0) return;
  const at = state.firstRendered + index;
  // Shift takes the block between here and where the last one was picked;
  // cmd or ctrl adds one on its own. Either way the message is not opened —
  // picking several and reading one are different intentions.
  if (event.shiftKey) {
    selectRange(at);
  } else if (event.metaKey || event.ctrlKey) {
    toggleSelected(at);
  } else {
    select(at);
    openSelected();
  }
});

/// Closes a dialog, saying so to whoever opened it: each dialog file listens
/// for `close` to tidy up after itself.
function closeDialog(sheet) {
  sheet.hidden = true;
  sheet.dispatchEvent(new Event("close"));
}

// -- start -----------------------------------------------------------------

async function start() {
  // The first run, with nothing to show yet, opens the assistant instead.
  const status = await invoke("setup_status").catch(() => null);
  if (status && status.first_run) {
    buildPool();
    statusBar.textContent = "setting up";
    await openSetup();
    return;
  }

  const accounts = await invoke("accounts");
  // Post is an addition: a Paperless that is down or not set up must not keep
  // mail from opening.
  state.postboxes = await invoke("paper_mailboxes").catch(() => []);
  countPostboxes();
  if (!accounts.length) {
    buildPool();
    if (state.postboxes.length) {
      await selectPostbox(state.postboxes[0]);
      return;
    }
    // Nothing to show and nowhere to go but settings, so go there.
    statusBar.textContent = "no accounts yet";
    await openSettings();
    return;
  }

  state.accounts = accounts;
  await loadProfiles();
  buildPool();
  // The profile shown last time, and its first account.
  const first = visibleAccounts()[0];
  if (first) await selectAccount(first);
  else if (visiblePostboxes().length) await selectPostbox(visiblePostboxes()[0]);
  else await selectAccount(accounts[0]);

  // What arrived since last time gets judged, quietly; and what tasks are
  // waiting for shows on the Assistant button.
  judgeUrgency();
  refreshAssistantBadge();

  const pending = await invoke("queue", { account: state.account });
  if (pending.length) say(`${pending.length} change(s) waiting for the next sync`);
}

// Once every script is in: the setup assistant lives in its own file, and a
// first run opens it before anything else.
document.addEventListener("DOMContentLoaded", () => {
  start().catch((err) => {
    statusBar.textContent = `failed to start: ${err}`;
  });
});

// -- settings ---------------------------------------------------------------

const settings = {
  sheet: el("settings"),
  list: el("settings-list"),
  form: el("settings-form"),
  report: el("settings-report"),
  passwordState: el("password-state"),
  oauthFields: el("oauth-fields"),
  passwordFields: el("password-fields"),
  // Which account the form is editing. Null means a new one.
  editing: null,
  accounts: [],
};

/// Sensible defaults for a brand-new account.
///
/// 993 and 465 because implicit TLS is what a modern provider offers, and a
/// form that opens on the right answer is one fewer thing to get wrong.
const NEW_ACCOUNT = {
  id: null,
  label: "",
  email: "",
  username: "",
  imap_host: "",
  imap_port: 993,
  imap_security: "tls",
  smtp_host: "",
  smtp_port: 465,
  smtp_security: "tls",
  auth_method: "app_password",
  oauth_provider: "microsoft",
  oauth_client_id: "",
  oauth_tenant: "",
  has_password: false,
  excluded_folders: [],
};

function formFields() {
  return Object.fromEntries(new FormData(settings.form).entries());
}

function fillForm(account) {
  settings.editing = account.id;
  settings.mode = "account";
  hideSettingsForms();
  settings.form.hidden = false;
  const f = settings.form;
  f.label.value = account.label ?? "";
  f.email.value = account.email ?? "";
  f.username.value = account.username ?? "";
  f.imap_host.value = account.imap_host ?? "";
  f.imap_port.value = account.imap_port ?? 993;
  f.imap_security.value = account.imap_security ?? "tls";
  f.smtp_host.value = account.smtp_host ?? "";
  f.smtp_port.value = account.smtp_port ?? "";
  f.smtp_security.value = account.smtp_security ?? "tls";
  f.auth_method.value = account.auth_method ?? "app_password";
  f.oauth_provider.value = account.oauth_provider ?? "microsoft";
  f.oauth_client_id.value = account.oauth_client_id ?? "";
  f.oauth_tenant.value = account.oauth_tenant ?? "";
  f.excluded_folders.value = (account.excluded_folders ?? []).join("\n");
  f.password.value = "";
  // A new account goes in the profile on screen, unless told otherwise.
  fillProfileSelect(
    f.profile,
    account.id === null ? activeProfile() : state.accounts.find((a) => a.id === account.id)?.profile_id ?? null,
  );

  // The field never shows a password, so it has to say whether there is one.
  settings.passwordState.textContent = account.has_password
    ? "A password is stored in the keychain."
    : "No password stored yet.";

  el("settings-delete").hidden = account.id === null;
  el("settings-import").hidden = account.id !== null;
  settings.report.hidden = true;
  setPageHead("account", account.id === null ? "New account" : account.email);
  syncAuthFields();
  renderSettingsList();
}

/// OAuth2 and app passwords need different fields, and showing both invites
/// filling in the wrong one.
function syncAuthFields() {
  const oauth = settings.form.auth_method.value === "oauth2";
  settings.oauthFields.hidden = !oauth;
  settings.passwordFields.hidden = oauth;
}

/// The buttons that open a new, empty form of each kind. Held here because
/// the list is redrawn from scratch and they are moved into it each time —
/// moving keeps their handlers; recreating would lose them.
const ADD_BUTTONS = {
  account: el("settings-add"),
  paper: el("settings-add-paper"),
  models: el("settings-models"),
  provider: el("settings-add-provider"),
};

/// The settings pages that are not one of the four forms.
const SETTINGS_PAGES = {
  profiles: el("profiles-page"),
  smart: el("smart-page"),
  keys: el("keys-page"),
  general: el("general-page"),
  diagnostics: el("diagnostics-page"),
};

/// What each section is, at the top of its page.
const PAGE_HEADS = {
  profiles: ["Profiles", "Private, one company, another — kept apart."],
  account: ["Account", "Where mail comes from and goes out through. Passwords go to the system keychain and are never shown again."],
  paper: ["Postal address", "Scanned post from Paperless-ngx, read beside your mail."],
  models: ["Models", "Which model reads scans and sorts mail, and where it runs."],
  provider: ["Model provider", "An Ollama elsewhere, or a hosted service with an OpenAI-compatible API."],
  smart: ["Smart mailboxes", "Saved searches that live in the sidebar."],
  keys: ["Encryption", "OpenPGP keys for signing and encrypting mail."],
  general: ["General", ""],
  diagnostics: ["Diagnostics", "A log to attach when something goes wrong."],
};

function setPageHead(mode, title) {
  const [defaultTitle, sub] = PAGE_HEADS[mode] ?? ["Settings", ""];
  el("settings-page-title").textContent = title ?? defaultTitle;
  el("settings-page-sub").textContent = sub;
}

/// A heading in the settings list, with its add button when it has one.
function settingsHeading(text, button, label) {
  const heading = document.createElement("div");
  heading.className = "sidebar-heading";
  const words = document.createElement("span");
  words.textContent = text;
  heading.append(words);
  if (button) {
    button.textContent = label;
    button.className = "";
    heading.append(button);
  }
  return heading;
}

function renderSettingsList() {
  settings.list.textContent = "";
  const onAccounts = settings.mode === "account";

  settings.list.append(
    navItem({ label: "Profiles", active: settings.mode === "profiles", onClick: () => showSettingsPage("profiles") }),
  );

  settings.list.append(settingsHeading("Accounts", ADD_BUTTONS.account, "+ Add"));
  for (const account of settings.accounts) {
    settings.list.append(
      navItem({
        label: account.email,
        active: onAccounts && account.id === settings.editing,
        onClick: () => fillForm(account),
      }),
    );
  }
  if (onAccounts && settings.editing === null) {
    settings.list.append(
      navItem({ label: "New account…", active: true, onClick: () => {} }),
    );
  }

  // Addresses under their own heading: they are configured like accounts and
  // are not accounts, and a single list pretending otherwise would be a list
  // where "Home" sits between two email addresses with no explanation.
  settings.list.append(settingsHeading("Postal addresses", ADD_BUTTONS.paper, "+ Add"));
  for (const address of paper.addresses) {
    settings.list.append(
      navItem({
        label: address.label || address.base_url,
        active: settings.mode === "paper" && address.id === paper.editing,
        onClick: () => fillPaper(address),
        // A token-less address is configured and unreadable, which is worth
        // seeing without opening it.
        title: address.has_token ? address.base_url : `${address.base_url} — no token stored`,
      }),
    );
  }
  if (settings.mode === "paper" && paper.editing === null) {
    settings.list.append(
      navItem({ label: "New address…", active: true, onClick: () => {} }),
    );
  }

  // Models: where they run is set up once and seldom visited.
  settings.list.append(settingsHeading("Models", ADD_BUTTONS.provider, "+ Add"));
  const jobs = ADD_BUTTONS.models;
  jobs.className = `nav-item${settings.mode === "models" ? " active" : ""}`;
  jobs.textContent = "";
  const jobsLabel = document.createElement("span");
  jobsLabel.className = "label";
  jobsLabel.textContent = "Model for each job";
  jobs.append(jobsLabel);
  settings.list.append(jobs);
  for (const provider of models.providers) {
    settings.list.append(
      navItem({
        label: provider.label,
        active: settings.mode === "provider" && provider.id === models.editing,
        onClick: () => fillProvider(provider),
        title:
          provider.kind === "openai" && !provider.has_key
            ? `${provider.base_url} — no key stored`
            : provider.base_url,
      }),
    );
  }
  if (settings.mode === "provider" && models.editing === null) {
    settings.list.append(
      navItem({ label: "New provider…", active: true, onClick: () => {} }),
    );
  }

  settings.list.append(settingsHeading("Mail"));
  settings.list.append(
    navItem({ label: "Smart mailboxes", active: settings.mode === "smart", onClick: () => showSettingsPage("smart") }),
    navItem({ label: "Encryption", active: settings.mode === "keys", onClick: () => showSettingsPage("keys") }),
  );

  settings.list.append(settingsHeading("kuverta"));
  settings.list.append(
    navItem({ label: "General", active: settings.mode === "general", onClick: () => showSettingsPage("general") }),
    navItem({ label: "Diagnostics", active: settings.mode === "diagnostics", onClick: () => showSettingsPage("diagnostics") }),
  );
}

/// One of the pages that is not a form: each file that owns one is told it
/// is being shown, so it can fill itself.
function showSettingsPage(mode) {
  settings.mode = mode;
  hideSettingsForms();
  SETTINGS_PAGES[mode].hidden = false;
  setPageHead(mode);
  renderSettingsList();
  SETTINGS_PAGES[mode].dispatchEvent(new Event("show"));
}

async function openSettings(page = null) {
  settings.accounts = await invoke("account_settings");
  // Post is an addition to the settings, not a prerequisite for them: a
  // failure to list addresses must not stop anyone editing an account.
  paper.addresses = await invoke("paper_mailboxes").catch(() => []);
  models.providers = await invoke("ai_providers").catch(() => []);
  await loadProfiles();
  settings.sheet.hidden = false;
  if (page) showSettingsPage(page);
  else fillForm(settings.accounts[0] ?? NEW_ACCOUNT);
}

async function closeSettings() {
  settings.sheet.hidden = true;
  // Accounts may have come or gone, so the window reloads rather than trusting
  // what it had before.
  state.accounts = await invoke("accounts");
  // Postal addresses too: one added, renamed or removed shows in the sidebar.
  state.postboxes = await invoke("paper_mailboxes").catch(() => []);
  // And profiles, which settings may have changed.
  await loadProfiles();
  countPostboxes();
  if (state.postbox) {
    const box = state.postboxes.find((p) => p.id === state.postbox.id);
    if (box) {
      await selectPostbox(box);
      return;
    }
    state.postbox = null;
  }
  if (!state.accounts.length) {
    if (state.postboxes.length) {
      await selectPostbox(state.postboxes[0]);
      return;
    }
    statusBar.textContent = "no accounts — add one in settings";
    return;
  }
  const still = visibleAccounts().find((a) => a.id === state.account);
  await selectAccount(still ?? visibleAccounts()[0] ?? state.accounts[0]);
}

/// The form's state, as the core wants it.
function accountInput() {
  const f = formFields();
  const smtpHost = f.smtp_host.trim();
  return {
    id: settings.editing,
    label: f.label.trim(),
    email: f.email.trim(),
    username: f.username.trim() || null,
    imap_host: f.imap_host.trim(),
    imap_port: Number(f.imap_port),
    imap_security: f.imap_security,
    smtp_host: smtpHost || null,
    smtp_port: smtpHost ? Number(f.smtp_port) : null,
    smtp_security: smtpHost ? f.smtp_security : null,
    auth_method: f.auth_method,
    oauth_provider: f.auth_method === "oauth2" ? f.oauth_provider : null,
    oauth_client_id: f.auth_method === "oauth2" ? f.oauth_client_id.trim() || null : null,
    oauth_tenant: f.auth_method === "oauth2" ? f.oauth_tenant.trim() || null : null,
    excluded_folders: f.excluded_folders
      .split("\n")
      .map((line) => line.trim())
      .filter(Boolean),
  };
}

settings.form.addEventListener("submit", async (event) => {
  event.preventDefault();
  const input = accountInput();
  const password = formFields().password;

  try {
    const id = await invoke("save_account", { input });

    // After the row, so a password is never stored for an account that failed
    // to save — which would leave a credential with nothing to use it.
    if (password) {
      await invoke("set_password", { email: input.email, password });
    }
    await invoke("set_account_profile", { account: id, profile: selectedProfile(settings.form.profile) });
    state.accounts = await invoke("accounts");

    settings.accounts = await invoke("account_settings");
    fillForm(settings.accounts.find((a) => a.id === id) ?? NEW_ACCOUNT);
    say("saved");
  } catch (err) {
    say(String(err), true);
  }
});

el("settings-add").onclick = () => fillForm(NEW_ACCOUNT);
el("settings-close").onclick = closeSettings;
settings.form.auth_method.addEventListener("change", syncAuthFields);

el("settings-delete").onclick = async () => {
  const account = settings.accounts.find((a) => a.id === settings.editing);
  if (!account) return;
  // Removing an account throws away its mail as well as its settings, and
  // there is no undo for that, so it asks.
  if (!confirm(`Remove ${account.email} and everything synced for it?`)) return;

  try {
    await invoke("delete_account", { id: account.id });
    settings.accounts = await invoke("account_settings");
    fillForm(settings.accounts[0] ?? NEW_ACCOUNT);
    say(`removed ${account.email}`);
  } catch (err) {
    say(String(err), true);
  }
};

el("settings-verify").onclick = async () => {
  const button = el("settings-verify");
  const input = accountInput();
  if (!input.email) {
    say("enter an email address first", true);
    return;
  }

  button.disabled = true;
  button.textContent = "Verifying…";
  settings.report.hidden = false;
  settings.report.textContent = "Connecting…";

  try {
    // Saved first, because verifying asks the core to connect to an account —
    // and an account it cannot see is one it cannot test.
    const id = await invoke("save_account", { input });
    const password = formFields().password;
    if (password) await invoke("set_password", { email: input.email, password });

    const report = await invoke("verify_account", { email: input.email });
    settings.report.textContent = describeReport(report);
    settings.accounts = await invoke("account_settings");
    settings.editing = id;
    renderSettingsList();
  } catch (err) {
    settings.report.textContent = String(err);
  } finally {
    button.disabled = false;
    button.textContent = "Verify";
  }
};

/// The report as text. Deliberately the same shape as `kuverta check`, so
/// the two do not have to be learned separately.
function describeReport(r) {
  const lines = [];
  lines.push(`IMAP  ${r.imap_target}`);
  lines.push(r.imap_ok ? "      connected and logged in" : `      FAILED — ${r.imap_error}`);

  if (r.imap_ok) {
    lines.push(`      ${r.extensions.map(([n, on]) => `${n} ${on ? "yes" : "no"}`).join("  ")}`);
    lines.push("");
    lines.push(`folders (${r.folders.length})`);
    for (const f of r.folders) {
      const use = f.special_use ?? "";
      const skip = f.excluded ? "  (not synced)" : "";
      lines.push(`  ${f.name.padEnd(30)} ${use.padEnd(10)} ${String(f.messages).padStart(6)}${skip}`);
    }
    lines.push("");
    lines.push("where mail will go");
    lines.push(`  sent      ${r.sent_folder ?? "(none found)"}`);
    lines.push(`  archived  ${r.archive_folder ?? "(none found)"}`);
    lines.push(`  deleted   ${r.trash_folder ?? "(none found)"}`);
    lines.push("");
    lines.push(`first sync would fetch ${r.total_messages} message(s)`);
  }

  if (r.smtp_target) {
    lines.push("");
    lines.push(`SMTP  ${r.smtp_target}`);
    lines.push(r.smtp_ok ? "      connected and authenticated" : `      FAILED — ${r.smtp_error}`);
  } else {
    lines.push("");
    lines.push("SMTP  not configured — this account cannot send");
  }

  for (const warning of r.warnings) {
    lines.push("");
    lines.push(`note: ${warning}`);
  }
  return lines.join("\n");
}

// -- postal addresses ----------------------------------------------------------

const paper = {
  form: el("paper-form"),
  tokenState: el("paper-token-state"),
  valueField: el("paper-selector-value"),
  // Which address the form is editing. Null means a new one.
  editing: null,
  addresses: [],
};

/// Where a new address starts: the dev stack's Paperless, every document.
///
/// "Every document" because one Paperless serving one address is the common
/// case, and a new address that matched nothing until a tag was typed would
/// look exactly like an address with no post.
const NEW_ADDRESS = {
  id: null,
  label: "",
  base_url: "http://localhost:8000",
  selector_kind: "everything",
  selector_value: null,
  has_token: false,
};

function fillPaper(address) {
  settings.mode = "paper";
  paper.editing = address.id;

  const f = paper.form;
  f.label.value = address.label ?? "";
  f.base_url.value = address.base_url ?? "";
  f.selector_kind.value = address.selector_kind ?? "everything";
  f.selector_value.value = address.selector_value ?? "";
  f.token.value = "";
  fillProfileSelect(f.profile, address.id === null ? activeProfile() : address.profile_id ?? null);

  // As with passwords: the field never shows the token, so it says whether
  // there is one — and where to get one, since that is not obvious.
  paper.tokenState.textContent = address.has_token
    ? "A token is stored in the keychain. Sign in again below if Paperless stops accepting it."
    : "No token stored yet. The button below opens Paperless at its profile page, where the API token is; sign in there, copy it, and paste it above.";

  el("paper-delete").hidden = address.id === null;
  hideSettingsForms();
  paper.form.hidden = false;
  setPageHead("paper", address.id === null ? "New postal address" : address.label || address.base_url);
  settings.report.hidden = true;
  syncSelectorField();
  renderSettingsList();
}

/// "All of them" needs no name, and an empty name field beside it invites
/// typing one that is then silently ignored.
function syncSelectorField() {
  paper.valueField.hidden = paper.form.selector_kind.value === "everything";
}

function paperInput() {
  const f = Object.fromEntries(new FormData(paper.form).entries());
  const kind = f.selector_kind;
  return {
    // Carried so that changing an address's URL or selector edits it, rather
    // than saving a second address and orphaning the first with its token.
    id: paper.editing,
    label: f.label.trim(),
    base_url: f.base_url.trim(),
    selector_kind: kind,
    selector_value: kind === "everything" ? null : f.selector_value.trim() || null,
  };
}

async function savePaper() {
  const id = await invoke("save_paper_mailbox", { input: paperInput() });
  const token = paper.form.token.value;
  // After the row, so a token is never stored for an address that failed to
  // save — a credential with nothing to use it.
  if (token) await invoke("set_paper_token", { id, token });
  await invoke("set_postbox_profile", { postbox: id, profile: selectedProfile(paper.form.profile) });
  paper.addresses = await invoke("paper_mailboxes");
  return id;
}

paper.form.addEventListener("submit", async (event) => {
  event.preventDefault();
  try {
    const id = await savePaper();
    fillPaper(paper.addresses.find((a) => a.id === id) ?? NEW_ADDRESS);
    say("saved");
  } catch (err) {
    say(String(err), true);
  }
});

/// Paperless keeps the API token on the signed-in user's profile page, which
/// is several clicks in and named differently from anything here. The link
/// goes to the address in the form rather than the saved one, so it works
/// while an address is still being typed — and through the login page, which
/// comes back to the profile when it is done.
el("paper-token-link").onclick = () => {
  const typed = paper.form.base_url.value.trim().replace(/\/+$/, "");
  if (!typed) {
    say("fill in the Paperless address first", true);
    return;
  }
  // An address typed without one is http, as the placeholder shows it.
  const base = /^https?:\/\//i.test(typed) ? typed : `http://${typed}`;
  invoke("open_external", { url: `${base}/accounts/profile/` }).catch((err) => say(String(err), true));
};

el("settings-add-paper").onclick = () => fillPaper(NEW_ADDRESS);
paper.form.selector_kind.addEventListener("change", syncSelectorField);

el("paper-delete").onclick = async () => {
  const address = paper.addresses.find((a) => a.id === paper.editing);
  if (!address) return;
  // Says what is and is not lost: the documents are Paperless's and stay
  // there. Only this client's view of them, and the stored token, go.
  const name = address.label || address.base_url;
  if (!confirm(`Remove ${name}? Its documents stay in Paperless; only the address and its stored token are forgotten here.`)) {
    return;
  }
  try {
    await invoke("delete_paper_mailbox", { id: address.id });
    paper.addresses = await invoke("paper_mailboxes");
    if (settings.accounts.length) fillForm(settings.accounts[0]);
    else fillPaper(NEW_ADDRESS);
    say(`removed ${name}`);
  } catch (err) {
    say(String(err), true);
  }
};

el("paper-verify").onclick = async () => {
  const button = el("paper-verify");
  button.disabled = true;
  button.textContent = "Verifying…";
  settings.report.hidden = false;
  settings.report.textContent = "Connecting…";

  try {
    // Saved first, as with accounts: the check runs against a stored address,
    // with the stored token.
    const id = await savePaper();
    paper.editing = id;
    renderSettingsList();
    const report = await invoke("paper_check", { id });
    settings.report.textContent = describePaperReport(report);
  } catch (err) {
    settings.report.textContent = String(err);
  } finally {
    button.disabled = false;
    button.textContent = "Verify";
  }
};

// -- models --------------------------------------------------------------------

const models = {
  form: el("ai-form"),
  providerForm: el("provider-form"),
  providers: [],
  // What each provider offers, by id, as a promise: two jobs on one provider
  // ask it once, and a provider's own page asks again.
  offered: new Map(),
  // Which provider the provider form is editing. Null means a new one.
  editing: null,
  // The kind of provider being edited: `ollama`, or `openai` for a service
  // with an OpenAI-compatible API.
  kind: "openai",
};

const LOCAL_PROVIDER = 1;
const TASKS = ["vision", "chat", "assistant"];
const TASK_NAMES = { vision: "reading scans", chat: "sorting mail", assistant: "the assistant" };

/// Where a new provider starts. Each address is the service's own documented
/// base for its OpenAI-compatible API.
const PRESETS = {
  deepseek: { kind: "openai", label: "DeepSeek", base_url: "https://api.deepseek.com" },
  openai: { kind: "openai", label: "OpenAI", base_url: "https://api.openai.com/v1" },
  openrouter: { kind: "openai", label: "OpenRouter", base_url: "https://openrouter.ai/api/v1" },
  custom: { kind: "openai", label: "", base_url: "https://" },
  ollama: { kind: "ollama", label: "Ollama", base_url: "http://" },
};

/// One form at a time in the sheet — see the note in styles.css on why each
/// form's hidden attribute needs a rule of its own.
function hideSettingsForms() {
  settings.form.hidden = true;
  paper.form.hidden = true;
  models.form.hidden = true;
  models.providerForm.hidden = true;
  for (const page of Object.values(SETTINGS_PAGES)) page.hidden = true;
  settings.report.hidden = true;
}

function formatSize(bytes) {
  return bytes >= 1e9 ? `${(bytes / 1e9).toFixed(1)} GB` : `${Math.round(bytes / 1e6)} MB`;
}

/// "sees images · 3.2 GB": what choosing a model needs to know about it.
function describeModel(model) {
  const sees = model.vision === true ? "sees images" : model.vision === false ? "text only" : "";
  return [sees, model.size_bytes ? formatSize(model.size_bytes) : ""].filter(Boolean).join(" · ");
}

function offeredBy(providerId, { fresh = false } = {}) {
  if (fresh || !models.offered.has(providerId)) {
    const asking = invoke("ai_models", { providerId });
    // A failure is asked again next time rather than remembered.
    asking.catch(() => {
      if (models.offered.get(providerId) === asking) models.offered.delete(providerId);
    });
    models.offered.set(providerId, asking);
  }
  return models.offered.get(providerId);
}

async function showModels() {
  settings.mode = "models";
  hideSettingsForms();
  models.form.hidden = false;
  setPageHead("models");
  renderSettingsList();
  try {
    const [providers, tasks] = await Promise.all([invoke("ai_providers"), invoke("ai_tasks")]);
    models.providers = providers;
    for (const task of tasks) {
      const select = models.form[`${task.task}_provider`];
      select.textContent = "";
      for (const provider of providers) select.append(new Option(provider.label, String(provider.id)));
      select.value = String(task.provider_id);
      models.form[`${task.task}_model`].value = task.model;
      fillOffered(task.task);
    }
    renderSettingsList();
  } catch (err) {
    say(String(err), true);
  }
}

/// Offers the chosen provider's models under a job's model field, and says
/// what the chosen model can do and what the job would send where.
async function fillOffered(task) {
  const providerField = models.form[`${task}_provider`];
  const providerId = Number(providerField.value);
  let offered = [];
  let failed = null;
  try {
    offered = await offeredBy(providerId);
  } catch (err) {
    failed = String(err);
  }
  if (Number(providerField.value) !== providerId) return; // changed while asking

  const list = el(`${task}-models`);
  list.textContent = "";
  for (const model of offered) {
    // An embedding model can be asked nothing, so it is no answer to either job.
    if (model.embedding) continue;
    const option = document.createElement("option");
    option.value = model.name;
    option.label = describeModel(model);
    list.append(option);
  }
  noteFor(task, offered, failed);
}

function noteFor(task, offered, failed) {
  const f = models.form;
  const provider = models.providers.find((p) => p.id === Number(f[`${task}_provider`].value));
  const name = f[`${task}_model`].value.trim();
  const model = offered.find((m) => m.name === name);
  const notes = [];

  if (failed) {
    notes.push(`Could not list ${provider?.label ?? "the provider"}'s models: ${failed}`);
  } else if (name && !model) {
    notes.push(
      provider?.kind === "ollama"
        ? `${name} is not on this Ollama yet — pull it with: ollama pull ${name}`
        : `${provider?.label} does not list ${name}.`,
    );
  }
  if (task === "vision" && model?.vision === false) {
    notes.push(`${name} cannot see images, so it cannot read scans.`);
  }
  if (task === "vision" && model && model.vision === null) {
    notes.push(`${provider.label} does not say whether ${name} can see images — Try them to find out.`);
  }
  if (provider && !provider.local) {
    notes.push(
      task === "vision"
        ? `Scans of your letters will be sent to ${provider.label}.`
        : task === "assistant"
          ? `What the assistant reads of your mail — whatever it searches for and opens — will be sent to ${provider.label}.`
          : `The sender, subject and start of each message will be sent to ${provider.label}.`,
    );
  }

  const note = f.querySelector(`[data-note="${task}"]`);
  note.textContent = notes.join(" ");
  note.classList.toggle("warn", Boolean(provider && !provider.local));
}

for (const task of TASKS) {
  models.form[`${task}_provider`].addEventListener("change", () => fillOffered(task));
  models.form[`${task}_model`].addEventListener("input", () => fillOffered(task));
}

models.form.addEventListener("submit", async (event) => {
  event.preventDefault();
  try {
    for (const task of TASKS) {
      await invoke("set_ai_task", {
        task,
        providerId: Number(models.form[`${task}_provider`].value),
        model: models.form[`${task}_model`].value.trim(),
      });
    }
    say("saved");
    await showModels();
  } catch (err) {
    say(String(err), true);
  }
});

/// Each job once, on a sample rather than anyone's mail — the only way to know
/// whether a service's model can see, since most services do not say.
el("ai-try").onclick = async () => {
  const button = el("ai-try");
  button.disabled = true;
  button.textContent = "Trying…";
  settings.report.hidden = false;
  settings.report.textContent = "Asking…";

  const lines = [];
  for (const task of TASKS) {
    const providerId = Number(models.form[`${task}_provider`].value);
    const model = models.form[`${task}_model`].value.trim();
    const provider = models.providers.find((p) => p.id === providerId);
    lines.push(`${TASK_NAMES[task]} — ${model} at ${provider?.label ?? providerId}`);
    try {
      const trial = await invoke("ai_try", { task, providerId, model });
      const seconds = (trial.latency_ms / 1000).toFixed(1);
      lines.push(`      ${trial.passed ? "ok" : "NOT OK"}: ${trial.verdict}, in ${seconds} s`);
      if (trial.reply) lines.push(`      it said: ${trial.reply}`);
    } catch (err) {
      lines.push(`      FAILED — ${err}`);
    }
    lines.push("");
    settings.report.textContent = lines.join("\n");
  }
  button.disabled = false;
  button.textContent = "Try them";
};

function fillProvider(provider) {
  settings.mode = "provider";
  models.editing = provider.id;
  models.kind = provider.kind;
  hideSettingsForms();
  models.providerForm.hidden = false;
  setPageHead("provider", provider.id === null ? "New model provider" : provider.label);

  const f = models.providerForm;
  // The service is a starting point for a new provider; an existing one is
  // edited by its address.
  el("provider-preset").hidden = provider.id !== null;
  f.label.value = provider.label ?? "";
  f.base_url.value = provider.base_url ?? "";
  f.key.value = "";
  syncProviderKind(provider);
  el("provider-delete").hidden = provider.id === null || provider.id === LOCAL_PROVIDER;
  el("provider-models").hidden = true;
  el("provider-models-status").textContent = provider.id === null ? "Save to see them." : "";
  renderSettingsList();
  if (provider.id !== null) listProviderModels(provider.id);
}

/// A key is for a hosted service; Ollama has none to ask for.
function syncProviderKind(provider) {
  const ollama = models.kind === "ollama";
  el("provider-key").hidden = ollama;
  el("provider-key-state").textContent = ollama
    ? "Ollama needs no key."
    : provider?.has_key
      ? "A key is stored in the keychain. It is sent only to this address."
      : "No key stored yet. Paste the API key from the service's dashboard; it is kept in the keychain.";
}

async function listProviderModels(id) {
  const table = el("provider-models");
  const status = el("provider-models-status");
  status.textContent = "Asking for its models…";
  try {
    const offered = await offeredBy(id, { fresh: true });
    if (settings.mode !== "provider" || models.editing !== id) return;
    const body = table.tBodies[0];
    body.textContent = "";
    for (const model of offered) {
      const row = body.insertRow();
      row.insertCell().textContent = model.name;
      row.insertCell().textContent = model.embedding
        ? "embeddings only"
        : model.vision === true
          ? "yes"
          : model.vision === false
            ? "no"
            : "not said";
      row.insertCell().textContent = model.size_bytes ? formatSize(model.size_bytes) : "";
    }
    table.hidden = offered.length === 0;
    const provider = models.providers.find((p) => p.id === id);
    status.textContent = offered.length
      ? `${offered.length} model${offered.length === 1 ? "" : "s"}`
      : provider?.kind === "ollama"
        ? "No models pulled yet. For reading scans: ollama pull qwen2.5vl:3b"
        : "The service lists no models for this key.";
  } catch (err) {
    if (settings.mode === "provider" && models.editing === id) {
      status.textContent = `Could not list its models: ${err}`;
    }
  }
}

async function saveProvider() {
  const f = models.providerForm;
  const id = await invoke("save_ai_provider", {
    input: {
      id: models.editing,
      kind: models.kind,
      label: f.label.value.trim(),
      base_url: f.base_url.value.trim(),
    },
  });
  const key = f.key.value.trim();
  // After the row, as with passwords and tokens: never a key with nothing to use it.
  if (key) await invoke("set_ai_key", { id, key });
  models.offered.delete(id);
  models.providers = await invoke("ai_providers");
  return id;
}

models.providerForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  try {
    const id = await saveProvider();
    fillProvider(models.providers.find((p) => p.id === id));
    say("saved");
  } catch (err) {
    say(String(err), true);
  }
});

models.providerForm.preset.addEventListener("change", () => {
  const preset = PRESETS[models.providerForm.preset.value];
  models.kind = preset.kind;
  models.providerForm.label.value = preset.label;
  models.providerForm.base_url.value = preset.base_url;
  syncProviderKind(null);
});

el("provider-delete").onclick = async () => {
  const provider = models.providers.find((p) => p.id === models.editing);
  if (!provider) return;
  if (!confirm(`Remove ${provider.label}? Its key is removed from the keychain, and any job using it goes back to the model on this computer.`)) {
    return;
  }
  try {
    await invoke("delete_ai_provider", { id: provider.id });
    models.offered.delete(provider.id);
    await showModels();
    say(`removed ${provider.label}`);
  } catch (err) {
    say(String(err), true);
  }
};

el("settings-models").onclick = showModels;
el("settings-add-provider").onclick = () => {
  models.providerForm.preset.value = "deepseek";
  fillProvider({ id: null, has_key: false, ...PRESETS.deepseek });
};

/// The Paperless preflight, as text — the same shape as the IMAP one above.
function describePaperReport(r) {
  const lines = [];
  lines.push("Paperless-ngx");
  lines.push(`      ${r.reachable ? "reachable" : "NOT reachable"}, ${r.authenticated ? "token accepted" : "token REJECTED"}`);
  lines.push("");
  lines.push(`${r.documents_total} document(s) in the instance`);
  lines.push(`${r.documents_matching} of them belong to this address`);

  const list = (names) => (names.length ? names.slice(0, 20).join(", ") + (names.length > 20 ? ", …" : "") : "none");
  lines.push("");
  lines.push(`tags            ${list(r.tags)}`);
  lines.push(`correspondents  ${list(r.correspondents)}`);

  // The notes are the point of the check: a selector that matches nothing
  // looks exactly like an address that has had no post.
  for (const note of r.notes) {
    lines.push("");
    lines.push(`note: ${note}`);
  }
  return lines.join("\n");
}
