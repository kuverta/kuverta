// Mailboxes on the server: making new ones, and deleting empty ones.
//
// Creating goes straight to the server — a folder is not mail, and there is
// nothing to hold back. Deleting is refused by the core unless the folder is
// empty on the server and is not one the account depends on; this file only
// asks first, and says what the core said when it refuses.

const contextMenu = el("context-menu");
const folderSheet = el("folder-sheet");
const folderForm = el("folder-form");

/// Mailboxes the account keeps its own kind of mail in. Offered for nothing
/// but opening; the core would refuse to delete them anyway.
function isEssential(folder) {
  if (folder.name.toUpperCase() === "INBOX") return true;
  if (["\\Sent", "\\Drafts", "\\Trash", "\\Junk", "\\Archive", "\\All"].includes(folder.special_use)) {
    return true;
  }
  if (folder.name === state.archive || folder.name === state.trash) return true;
  // By name, for servers that mark nothing — the same names the core knows.
  const leaf = folder.name.split("/").pop().toLowerCase();
  return ESSENTIAL_NAMES.includes(leaf);
}

const ESSENTIAL_NAMES = [
  "sent", "sent items", "sent messages", "sent mail", "inbox.sent", "gesendet",
  "gesendete objekte", "gesendete elemente", "drafts", "entwürfe", "entwuerfe",
  "inbox.drafts", "junk", "spam", "junk e-mail", "bulk mail", "trash", "deleted items",
  "deleted messages", "inbox.trash", "papierkorb", "gelöschte objekte", "gelöschte elemente",
  "archive", "archiv", "archived", "all mail",
];

/// A small menu at the pointer. `items` are `[label, action, { danger, disabled }]`,
/// or `null` for a divider.
function showMenu(event, items) {
  contextMenu.textContent = "";
  for (const item of items) {
    if (!item) {
      contextMenu.append(document.createElement("hr"));
      continue;
    }
    const [label, action, { danger = false, disabled = false, title = "" } = {}] = item;
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = label;
    button.disabled = disabled;
    if (title) button.title = title;
    if (danger) button.className = "danger";
    button.onclick = () => {
      contextMenu.hidden = true;
      action();
    };
    contextMenu.append(button);
  }
  contextMenu.hidden = false;
  // Kept on screen: a menu opened near the bottom edge opens upwards.
  const { innerWidth, innerHeight } = window;
  const rect = contextMenu.getBoundingClientRect();
  contextMenu.style.left = `${Math.min(event.clientX, innerWidth - rect.width - 8)}px`;
  contextMenu.style.top = `${Math.min(event.clientY, innerHeight - rect.height - 8)}px`;
}

document.addEventListener("mousedown", (event) => {
  if (!contextMenu.hidden && !contextMenu.contains(event.target)) contextMenu.hidden = true;
});
window.addEventListener("blur", () => (contextMenu.hidden = true));

function showFolderMenu(event, folder) {
  const essential = isEssential(folder);
  showMenu(event, [
    [`New mailbox inside “${folder.label}”…`, () => openFolderDialog(folder.name)],
    null,
    [
      `Delete “${folder.label}”…`,
      () => deleteFolder(folder),
      {
        danger: true,
        disabled: essential,
        title: essential ? "The account keeps its mail of this kind here" : "",
      },
    ],
  ]);
}

function openFolderDialog(parent = null) {
  if (state.postbox || !state.email) {
    say("choose a mail account first", true);
    return;
  }
  folderForm.reset();
  el("folder-form-account").textContent = `On ${state.email}`;
  const select = folderForm.parent;
  select.textContent = "";
  select.append(new Option("Nowhere — at the top", ""));
  for (const folder of state.folders) select.append(new Option(folder.name, folder.name));
  select.value = parent ?? "";
  folderSheet.hidden = false;
  folderForm.name.focus();
}

folderForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  const name = folderForm.name.value.trim();
  const parent = folderForm.parent.value || null;
  const button = folderForm.querySelector("[type=submit]");
  button.disabled = true;
  try {
    const created = await invoke("create_folder", { email: state.email, name, parent });
    closeDialog(folderSheet);
    say(`created ${created.name}`);
    await refreshSidebar();
  } catch (err) {
    say(String(err), true);
  } finally {
    button.disabled = false;
  }
});

el("folder-cancel").onclick = () => closeDialog(folderSheet);
el("folder-add").onclick = () => openFolderDialog(null);

async function deleteFolder(folder) {
  if (folder.total > 0) {
    say(`${folder.label} still has ${folder.total} message${folder.total === 1 ? "" : "s"} — move them out first`, true);
    return;
  }
  if (!confirm(`Delete the mailbox “${folder.name}” from the server?`)) return;
  try {
    await invoke("delete_folder", { email: state.email, folder: folder.id });
    if (state.filter.folder === folder.id) state.filter = { ...state.filter, folder: null };
    say(`deleted ${folder.name}`);
    await reload();
  } catch (err) {
    say(String(err), true);
  }
}
