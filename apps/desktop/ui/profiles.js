// Profiles: accounts and postal addresses kept apart by what they are for —
// private, one company, another.
//
// The window shows one profile at a time, or all of them; the choice is kept
// across restarts. Accounts and addresses in no profile show under All only.
// The core keeps the profiles and who is in which; this draws the switcher,
// the settings page, and the Profile field on the account and address forms.

const profilesPage = SETTINGS_PAGES.profiles;

/// The profiles as the core last said, in their order.
state.profiles = [];

/// The profile on screen: an id, or null for all of them.
function activeProfile() {
  try {
    const stored = Number(localStorage.getItem("profile"));
    return state.profiles.some((p) => p.id === stored) ? stored : null;
  } catch {
    return null;
  }
}

function setActiveProfile(id) {
  try {
    localStorage.setItem("profile", id === null ? "" : String(id));
  } catch {
    // Lasts until the window closes.
  }
}

async function loadProfiles() {
  state.profiles = await invoke("profiles").catch(() => []);
}

/// The accounts the active profile shows.
function visibleAccounts() {
  const profile = activeProfile();
  return profile === null ? state.accounts : state.accounts.filter((a) => a.profile_id === profile);
}

function visiblePostboxes() {
  const profile = activeProfile();
  return profile === null ? state.postboxes : state.postboxes.filter((p) => p.profile_id === profile);
}

// -- the switcher -------------------------------------------------------------------

/// All · Private · Work, above the accounts. Only there once there are
/// profiles to choose between.
function renderProfileBar() {
  const bar = el("profile-bar");
  bar.textContent = "";
  bar.hidden = state.profiles.length === 0;
  if (bar.hidden) return;
  const active = activeProfile();
  const pill = (label, id, count) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = `profile-pill${active === id ? " active" : ""}`;
    button.textContent = label;
    button.title = id === null ? "Every account and address" : `${count} account${count === 1 ? "" : "s"} and address${count === 1 ? "" : "es"}`;
    button.onclick = () => switchProfile(id);
    bar.append(button);
  };
  pill("All", null);
  for (const profile of state.profiles) {
    pill(profile.name, profile.id, profile.accounts.length + profile.postboxes.length);
  }
}

/// Shows another profile, and moves off an account or address it does not have.
async function switchProfile(id) {
  setActiveProfile(id);
  const accounts = visibleAccounts();
  const postboxes = visiblePostboxes();
  if (state.postbox && postboxes.some((p) => p.id === state.postbox.id)) {
    await refreshSidebar();
    return;
  }
  if (!state.postbox && accounts.some((a) => a.id === state.account)) {
    await refreshSidebar();
    return;
  }
  if (accounts.length) await selectAccount(accounts[0]);
  else if (postboxes.length) await selectPostbox(postboxes[0]);
  else {
    await refreshSidebar();
    say("nothing is in this profile yet — add accounts to it in Settings → Profiles");
  }
}

// -- the Profile field on the account and address forms -------------------------------

/// Fills a Profile select, and hides its label when there are no profiles to pick.
function fillProfileSelect(select, current) {
  select.textContent = "";
  select.append(new Option("No profile", ""));
  for (const profile of state.profiles) select.append(new Option(profile.name, String(profile.id)));
  select.value = current ? String(current) : "";
  select.closest("label").hidden = state.profiles.length === 0;
}

function selectedProfile(select) {
  return select.value ? Number(select.value) : null;
}

// -- settings: the page ------------------------------------------------------------------

async function fillProfilesPage() {
  await loadProfiles();
  const accounts = await invoke("accounts").catch(() => []);
  const postboxes = await invoke("paper_mailboxes").catch(() => []);
  profilesPage.textContent = "";

  const lead = document.createElement("p");
  lead.className = "lead";
  lead.textContent =
    "Keep private mail and each company's mail apart. The switcher above the sidebar shows one profile at a time, or all of them. An account or address in no profile shows under All only.";
  profilesPage.append(lead);

  const list = document.createElement("section");
  list.className = "card";
  list.innerHTML = '<h3>Profiles</h3><div class="item-list" data-empty="No profiles yet. Add one below."></div>';
  const items = list.querySelector(".item-list");
  state.profiles.forEach((profile, at) => items.append(profileItem(profile, at)));

  const add = document.createElement("form");
  add.className = "card-actions";
  add.innerHTML = '<input name="name" placeholder="e.g. Private, Company 1" spellcheck="false"><button type="submit" class="primary">Add profile</button>';
  add.addEventListener("submit", async (event) => {
    event.preventDefault();
    try {
      await invoke("save_profile", { id: null, name: add.name.value });
      await fillProfilesPage();
      renderProfileBar();
    } catch (err) {
      say(String(err), true);
    }
  });
  list.append(add);
  profilesPage.append(list);

  if (!state.profiles.length) return;

  // Who is in which, in one place.
  const members = document.createElement("section");
  members.className = "card";
  members.innerHTML = "<h3>What is in each</h3>";
  const table = document.createElement("div");
  table.className = "item-list";
  const row = (label, sub, current, save) => {
    const item = document.createElement("div");
    item.className = "item";
    const text = document.createElement("div");
    text.className = "grow";
    const title = document.createElement("div");
    title.className = "title";
    title.textContent = label;
    const hint = document.createElement("div");
    hint.className = "sub";
    hint.textContent = sub;
    text.append(title, hint);
    const select = document.createElement("select");
    select.append(new Option("No profile", ""));
    for (const profile of state.profiles) select.append(new Option(profile.name, String(profile.id)));
    select.value = current ? String(current) : "";
    select.addEventListener("change", async () => {
      try {
        await save(select.value ? Number(select.value) : null);
        await loadProfiles();
        renderProfileBar();
      } catch (err) {
        say(String(err), true);
      }
    });
    item.append(text, select);
    table.append(item);
  };
  for (const account of accounts) {
    row(account.email, "mail account", account.profile_id, (profile) =>
      invoke("set_account_profile", { account: account.id, profile }),
    );
  }
  for (const postbox of postboxes) {
    row(postbox.label, `postal address · ${postbox.base_url}`, postbox.profile_id, (profile) =>
      invoke("set_postbox_profile", { postbox: postbox.id, profile }),
    );
  }
  members.append(table);
  profilesPage.append(members);
}

function profileItem(profile, at) {
  const item = document.createElement("div");
  item.className = "item";
  const name = document.createElement("input");
  name.value = profile.name;
  name.className = "grow";
  name.spellcheck = false;
  name.title = "Rename by typing; it is saved when you leave the field";
  name.addEventListener("change", async () => {
    try {
      await invoke("save_profile", { id: profile.id, name: name.value });
      await loadProfiles();
      renderProfileBar();
    } catch (err) {
      name.value = profile.name;
      say(String(err), true);
    }
  });
  const count = document.createElement("span");
  count.className = "chip";
  const n = profile.accounts.length + profile.postboxes.length;
  count.textContent = `${n} in it`;
  const move = (label, delta, disabled) => {
    const b = document.createElement("button");
    b.type = "button";
    b.textContent = label;
    b.disabled = disabled;
    b.title = delta < 0 ? "Move up" : "Move down";
    b.onclick = async () => {
      const ids = state.profiles.map((p) => p.id);
      [ids[at], ids[at + delta]] = [ids[at + delta], ids[at]];
      await invoke("reorder_profiles", { ids });
      await fillProfilesPage();
      renderProfileBar();
    };
    return b;
  };
  const remove = document.createElement("button");
  remove.type = "button";
  remove.className = "danger";
  remove.textContent = "Delete";
  remove.onclick = async () => {
    if (!confirm(`Delete the profile “${profile.name}”? Its accounts and addresses stay, in no profile.`)) return;
    await invoke("delete_profile", { id: profile.id });
    if (activeProfile() === profile.id) setActiveProfile(null);
    await fillProfilesPage();
    renderProfileBar();
  };
  item.append(name, count, move("↑", -1, at === 0), move("↓", 1, at === state.profiles.length - 1), remove);
  return item;
}

profilesPage.addEventListener("show", fillProfilesPage);
