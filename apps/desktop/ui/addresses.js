// Address suggestions: who to write to, from the mail there already is.
//
// Typing in To, Cc or Bcc offers the people the account has written to and
// heard from — written-to first, then by how often and how lately — whose
// name or address starts with what is typed at the caret: "eri", "muster",
// "example.de" all find Erika Mustermann <erika@example.de>. ↓ and ↑ choose,
// Enter or Tab takes one, Escape closes the list without discarding the
// message.
//
// The whole address book comes once when compose opens and is filtered here,
// as each key is typed, without asking the core again. It is the account's
// own mail read back, nothing more: a newsletter's sender is not in it, and
// neither is anyone who only ever wrote from the Trash.

const addressBook = { account: null, contacts: [], loadedAt: 0, loading: null };

/// How long a loaded address book is used before it is asked for again.
const ADDRESS_BOOK_FRESH_MS = 5 * 60 * 1000;

/// Loads the address book for the open account, unless it is fresh.
function loadAddressBook({ force = false } = {}) {
  const account = state.account;
  if (account === null) return Promise.resolve();
  const fresh = addressBook.account === account && Date.now() - addressBook.loadedAt < ADDRESS_BOOK_FRESH_MS;
  if (fresh && !force) return Promise.resolve();
  if (addressBook.loading && addressBook.account === account && !force) return addressBook.loading;
  addressBook.account = account;
  addressBook.loading = invoke("address_book", { account })
    .then((contacts) => {
      if (addressBook.account !== account) return;
      addressBook.contacts = contacts;
      addressBook.loadedAt = Date.now();
    })
    .catch(() => {
      // No suggestions is a field that still works.
    })
    .finally(() => {
      addressBook.loading = null;
    });
  return addressBook.loading;
}

/// Says the book is out of date: something was sent, or mail arrived.
function addressBookChanged() {
  addressBook.loadedAt = 0;
}

/// Where the address the caret is in starts and ends: between the commas
/// around it that are not inside a quoted name.
function tokenAt(value, caret) {
  let start = 0;
  let quoted = false;
  for (let at = 0; at < caret; at += 1) {
    if (value[at] === '"') quoted = !quoted;
    else if (value[at] === "," && !quoted) start = at + 1;
  }
  let end = value.length;
  quoted = false;
  for (let at = start; at < value.length; at += 1) {
    if (value[at] === '"') quoted = !quoted;
    else if (value[at] === "," && !quoted && at >= caret) {
      end = at;
      break;
    }
  }
  return { start, end, text: value.slice(start, end).trim() };
}

/// The contacts `typed` could be the start of, most likely first, leaving
/// out who is already in the field.
function suggestionsFor(typed, already) {
  const needle = typed.toLowerCase();
  if (!needle || needle.includes("<")) return [];
  const starts = (text) => (text ?? "").toLowerCase().startsWith(needle);
  const found = [];
  for (const contact of addressBook.contacts) {
    if (already.has(contact.address)) continue;
    const [local, domain] = contact.address.split("@");
    const words = (contact.name ?? "").split(/[\s.,'"()-]+/);
    const byStart = starts(contact.address) || starts(contact.name);
    if (byStart || words.some(starts) || starts(domain) || local.split(/[._-]/).some(starts)) {
      found.push({ contact, byStart });
      if (found.length >= 40) break;
    }
  }
  // What starts with the typed text first; within each, the book's order.
  found.sort((a, b) => Number(b.byStart) - Number(a.byStart));
  return found.slice(0, 6).map((f) => f.contact);
}

/// How a contact goes into the field: with their name when there is one,
/// quoted when the name has what would split it.
function formatAddress(contact) {
  const name = (contact.name ?? "").trim();
  if (!name || name.toLowerCase() === contact.address) return contact.address;
  const safe = /[",;<>@]/.test(name) ? `"${name.replace(/"/g, "")}"` : name;
  return `${safe} <${contact.address}>`;
}

/// The addresses already in a field, lowercased.
function addressesIn(value) {
  const found = new Set();
  for (const match of value.matchAll(/[^\s<>,"]+@[^\s<>,"]+/g)) found.add(match[0].toLowerCase());
  return found;
}

/// Gives an address field a list of suggestions.
function suggestAddresses(field) {
  const list = document.createElement("div");
  list.className = "address-suggestions";
  list.setAttribute("role", "listbox");
  list.setAttribute("aria-label", t("Suggested addresses"));
  list.hidden = true;
  field.after(list);
  field.setAttribute("aria-autocomplete", "list");
  field.setAttribute("aria-expanded", "false");
  let shown = [];
  let active = 0;

  const close = () => {
    list.hidden = true;
    shown = [];
    field.setAttribute("aria-expanded", "false");
  };

  const take = (contact) => {
    const token = tokenAt(field.value, field.selectionStart ?? field.value.length);
    const before = field.value.slice(0, token.start);
    const after = field.value.slice(token.end).replace(/^\s*,?\s*/, "");
    const inserted = `${before ? `${before.trimEnd()} ` : ""}${formatAddress(contact)}, `;
    field.value = inserted + after;
    field.setSelectionRange(inserted.length, inserted.length);
    close();
    field.focus();
    // The envelope and the keys for encryption follow the recipients.
    field.dispatchEvent(new Event("change"));
  };

  const draw = () => {
    list.textContent = "";
    shown.forEach((contact, at) => {
      const option = document.createElement("div");
      option.className = `address-suggestion${at === active ? " active" : ""}`;
      option.setAttribute("role", "option");
      option.setAttribute("aria-selected", String(at === active));
      option.id = `${field.id}-suggestion-${at}`;
      const name = document.createElement("span");
      name.className = "name";
      // textContent: names are whatever the sender wrote.
      name.textContent = contact.name ?? contact.address;
      option.append(name);
      if (contact.name) {
        const address = document.createElement("span");
        address.className = "address";
        address.textContent = contact.address;
        option.append(address);
      }
      // Pressed rather than clicked: a click would take focus from the field
      // first and close the list under the pointer.
      option.addEventListener("mousedown", (event) => {
        event.preventDefault();
        take(contact);
      });
      list.append(option);
    });
    list.hidden = shown.length === 0;
    field.setAttribute("aria-expanded", String(!list.hidden));
    if (!list.hidden) {
      field.setAttribute("aria-activedescendant", `${field.id}-suggestion-${active}`);
      list.style.left = `${field.offsetLeft}px`;
      list.style.top = `${field.offsetTop + field.offsetHeight + 2}px`;
      list.style.width = `${field.offsetWidth}px`;
    } else {
      field.removeAttribute("aria-activedescendant");
    }
  };

  const update = () => {
    const token = tokenAt(field.value, field.selectionStart ?? field.value.length);
    shown = suggestionsFor(token.text, addressesIn(field.value.slice(0, token.start) + field.value.slice(token.end)));
    active = 0;
    draw();
  };

  field.addEventListener("input", () => {
    if (addressBook.account !== state.account || !addressBook.loadedAt) {
      loadAddressBook().then(() => document.activeElement === field && update());
    }
    update();
  });
  field.addEventListener("blur", close);
  // Before compose's own keys: Escape here closes the list, not the message.
  field.addEventListener("keydown", (event) => {
    if (list.hidden || event.metaKey || event.ctrlKey || event.altKey) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      active = (active + (event.key === "ArrowDown" ? 1 : shown.length - 1)) % shown.length;
      draw();
    } else if (event.key === "Enter" || (event.key === "Tab" && !event.shiftKey)) {
      event.preventDefault();
      event.stopPropagation();
      take(shown[active]);
    } else if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      close();
    }
  });
}

for (const field of [compose.to, compose.cc, compose.bcc]) suggestAddresses(field);
