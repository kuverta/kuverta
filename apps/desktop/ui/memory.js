// What the assistant remembers about each account: its notes, to read,
// correct, add to or forget, and whether they travel to the person's other
// devices through the mail server, encrypted to their own key.

const memoryPage = SETTINGS_PAGES.memory;
const memory = { account: null };

function memoryLayout() {
  memoryPage.innerHTML = `
    <p class="lead">${t("Short notes the assistant keeps about an account — who someone is, where a kind of mail goes, how you like things done — so it need not look them up again. They go with every question to the assistant's model, so a few short ones save more than many long ones.")}</p>
    <section class="card">
      <label>${t("Account")}<select data-el="account"></select></label>
    </section>
    <section class="card">
      <h3>${t("Notes")} <span class="hint" data-el="count"></span></h3>
      <div class="item-list" data-list="notes" data-empty="${t("No notes yet. The assistant keeps one when you tell it something worth remembering, or add one here.")}"></div>
      <form data-form="add" class="memory-add" autocomplete="off">
        <input name="text" maxlength="300" placeholder="${t("e.g. Invoices from the electricity company go to the folder Rechnungen")}">
        <button type="submit" class="primary">${t("Add")}</button>
      </form>
      <p class="hint">${t("The assistant keeps notes only from what you tell it, never because a message asks it to; each one it keeps shows in the chat with a way to forget it.")}</p>
    </section>
    <section class="card">
      <h3>${t("On your other devices")}</h3>
      <label class="check"><input type="checkbox" data-el="sync"> ${t("Sync the notes through this account's mail server")}</label>
      <p class="hint">${t("Kept as one message in a folder named “Kuverta Memory” that kuverta does not show as mail, signed with this account's OpenPGP key and encrypted to it alone: the server, and anyone who reads it there, sees only that the message exists. Another device reads the notes once it has the same key. They sync with the mail.")}</p>
      <p class="hint" data-el="sync-status"></p>
      <div class="card-actions">
        <button type="button" data-el="sync-now">${t("Sync now")}</button>
        <button type="button" data-el="keys">${t("Encryption settings")}</button>
      </div>
    </section>
  `;
  const accounts = memoryPage.querySelector("[data-el=account]");
  accounts.addEventListener("change", () => {
    memory.account = Number(accounts.value);
    fillMemory();
  });
  memoryPage.querySelector("[data-form=add]").addEventListener("submit", async (event) => {
    event.preventDefault();
    const input = event.target.elements.text;
    if (!input.value.trim()) return;
    try {
      await invoke("add_memory", { account: memory.account, text: input.value });
      input.value = "";
      await fillMemory();
    } catch (err) {
      say(String(err), true);
    }
  });
  memoryPage.querySelector("[data-el=sync]").addEventListener("change", async (event) => {
    try {
      await invoke("set_memory_sync", { account: memory.account, enabled: event.target.checked });
      if (event.target.checked) await syncMemoryNow();
      else await fillMemory();
    } catch (err) {
      event.target.checked = !event.target.checked;
      say(String(err), true);
    }
  });
  memoryPage.querySelector("[data-el=sync-now]").onclick = syncMemoryNow;
  memoryPage.querySelector("[data-el=keys]").onclick = () => showSettingsPage("keys");
}

function memoryEmail() {
  return state.accounts.find((account) => account.id === memory.account)?.email;
}

async function syncMemoryNow() {
  const button = memoryPage.querySelector("[data-el=sync-now]");
  button.disabled = true;
  memoryPage.querySelector("[data-el=sync-status]").textContent = t("Syncing…");
  try {
    const report = await invoke("sync_memory", { email: memoryEmail() });
    say(
      report.received
        ? t("notes synced — {count} changed by another device", { count: report.received })
        : t("notes synced"),
    );
  } catch (err) {
    say(String(err), true);
  } finally {
    button.disabled = false;
    await fillMemory();
  }
}

function noteItem(note) {
  const item = document.createElement("div");
  item.className = "item";
  const text = document.createElement("input");
  text.className = "grow memory-text";
  text.value = note.text;
  text.maxLength = 300;
  text.setAttribute("aria-label", t("Note"));
  const save = document.createElement("button");
  save.type = "button";
  save.textContent = t("Save");
  save.hidden = true;
  text.addEventListener("input", () => {
    save.hidden = text.value.trim() === note.text;
  });
  save.onclick = async () => {
    try {
      await invoke("update_memory", { account: memory.account, id: note.id, text: text.value });
      await fillMemory();
    } catch (err) {
      say(String(err), true);
    }
  };
  text.addEventListener("keydown", (event) => {
    if (event.key === "Enter" && !save.hidden) save.click();
  });
  const forget = document.createElement("button");
  forget.type = "button";
  forget.className = "danger";
  forget.textContent = t("Forget");
  forget.onclick = async () => {
    try {
      await invoke("forget_memory", { account: memory.account, id: note.id });
      await fillMemory();
    } catch (err) {
      say(String(err), true);
    }
  };
  item.append(text, save, forget);
  return item;
}

async function fillMemory() {
  if (!memoryPage.firstElementChild) memoryLayout();
  const accounts = memoryPage.querySelector("[data-el=account]");
  accounts.textContent = "";
  for (const account of state.accounts) accounts.append(new Option(account.email, account.id));
  if (!state.accounts.some((account) => account.id === memory.account)) {
    memory.account = state.account ?? state.accounts[0]?.id ?? null;
  }
  if (memory.account == null) return;
  accounts.value = memory.account;

  try {
    const [notes, sync] = await Promise.all([
      invoke("memories", { account: memory.account }),
      invoke("memory_sync_status", { account: memory.account }),
    ]);
    const list = memoryPage.querySelector("[data-list=notes]");
    list.textContent = "";
    for (const note of notes) list.append(noteItem(note));
    memoryPage.querySelector("[data-el=count]").textContent = notes.length ? t("{count} of 60", { count: notes.length }) : "";

    const box = memoryPage.querySelector("[data-el=sync]");
    box.checked = sync.enabled;
    box.disabled = !sync.enabled && !!sync.unavailable;
    memoryPage.querySelector("[data-el=sync-now]").hidden = !sync.enabled;
    memoryPage.querySelector("[data-el=keys]").hidden = !sync.unavailable;
    const status = memoryPage.querySelector("[data-el=sync-status]");
    status.classList.toggle("bad", !!(sync.unavailable || sync.last_error));
    if (sync.unavailable) status.textContent = sync.unavailable;
    else if (!sync.enabled) status.textContent = t("Encrypted to key {id}.", { id: sync.key_id });
    else if (sync.last_error) status.textContent = t("The last sync failed: {error}", { error: sync.last_error });
    else if (sync.synced_at)
      status.textContent = t("Encrypted to key {id}. Last synced {when}.", {
        id: sync.key_id,
        when: new Date(sync.synced_at * 1000).toLocaleString(),
      });
    else status.textContent = t("Encrypted to key {id}. Not synced yet.", { id: sync.key_id });
  } catch (err) {
    say(String(err), true);
  }
}

memoryPage.addEventListener("show", fillMemory);
