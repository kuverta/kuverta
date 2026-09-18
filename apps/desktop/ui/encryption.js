// OpenPGP: what an opened message's signature and encryption came to, the
// Sign and Encrypt switches in compose, and the keys in settings.
//
// The core does the cryptography and decides what is possible — which
// recipients have keys, whether the sender can sign. This says it, in words:
// a padlock that means "encrypted to someone, maybe not the person you think"
// is worse than no padlock.

const securityNote = el("reading-security");
const keysPage = SETTINGS_PAGES.keys;

/// What each opened message's security came to, by id, so a reply to an
/// encrypted message can start out encrypted too.
const securityOf = new Map();

// -- reading ------------------------------------------------------------------

function showSecurity(detail) {
  securityNote.hidden = true;
  securityNote.className = "security-note";
  securityNote.textContent = "";
  if (!detail) return;
  securityOf.set(detail.id, detail.security ?? null);

  const lines = [];
  let bad = false;
  const security = detail.security;
  if (security) {
    if (security.encrypted) {
      if (security.decrypted) lines.push("Encrypted — decrypted with your key.");
      else {
        bad = true;
        lines.push(`Encrypted, and kuverta could not decrypt it: ${security.error ?? "no reason given"}.`);
      }
    }
    const signature = security.signature;
    if (security.signed && signature) {
      const who = signature.signer ?? (signature.key_id ? `key ${signature.key_id}` : "an unknown key");
      if (signature.state === "valid") {
        lines.push(
          signature.signer_matches_sender === false
            ? `Signed by ${who} — the signature is valid, but that is not who the message says it is from.`
            : `Signed by ${who} — the signature is valid.`,
        );
        if (signature.signer_matches_sender === false) bad = true;
      } else if (signature.state === "invalid") {
        bad = true;
        lines.push(`The signature from ${who} does not match: the message may have been changed on the way.`);
      } else {
        lines.push(`Signed with a key kuverta does not have (${signature.key_id ?? "unknown"}). Import the sender's key to check it.`);
      }
    } else if (security.error && security.decrypted) {
      lines.push(security.error);
    }
  }

  if (!lines.length && !detail.pgp_keys_attached) return;
  const text = document.createElement("span");
  text.textContent = lines.join(" ") || "This message carries an OpenPGP key.";
  securityNote.append(text);

  if (detail.pgp_keys_attached) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = "Import the attached key";
    button.onclick = async () => {
      try {
        const report = await invoke("pgp_import_from_message", { account: state.account, id: detail.id });
        say(describeImport(report), report.errors.length > 0 && !report.imported.length);
      } catch (err) {
        say(String(err), true);
      }
    };
    securityNote.append(button);
  }
  if (bad && security?.encrypted && !security.decrypted) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = "Encryption settings";
    button.onclick = () => openSettings("keys");
    securityNote.append(button);
  }
  securityNote.classList.toggle("bad", bad);
  securityNote.hidden = false;
}

function describeImport(report) {
  const parts = [];
  for (const key of report.imported) {
    const who = key.user_ids[0] ?? key.fingerprint;
    parts.push(`${key.updated ? "updated" : "imported"} ${key.secret ? "your key" : "the key of"} ${who}`);
  }
  if (report.errors.length) parts.push(`not imported: ${report.errors.join("; ")}`);
  return parts.join(" · ") || "no keys found";
}

// -- compose ------------------------------------------------------------------

const composeSecurity = el("compose-security");
const signBox = el("compose-sign");
const encryptBox = el("compose-encrypt");
const securityHint = el("compose-security-note");

/// Whether the person has touched the switches in this draft; until then
/// they follow the message being replied to.
let securityChosen = false;
signBox.addEventListener("change", () => (securityChosen = true));
encryptBox.addEventListener("change", () => {
  securityChosen = true;
  refreshComposeSecurity();
});

/// Called whenever the recipients change (see `refreshEnvelope`).
async function refreshComposeSecurity() {
  if (compose.pane.hidden || !state.email) return;
  const recipients = [...addresses(compose.to), ...addresses(compose.cc)];
  let keys;
  try {
    keys = await invoke("pgp_recipients", { email: state.email, recipients });
  } catch {
    composeSecurity.hidden = true;
    return;
  }
  // Nothing to offer to someone with no key of their own and no correspondents'.
  const anyKeys = keys.sender_key || keys.recipients.some((r) => r.fingerprint);
  composeSecurity.hidden = !anyKeys;
  if (!anyKeys) return;

  signBox.disabled = !keys.can_sign;
  signBox.parentElement.title = keys.can_sign ? "Sign with your key, so recipients can tell it is from you" : keys.sign_problem ?? "";
  if (!keys.can_sign) signBox.checked = false;

  const hasBcc = addresses(compose.bcc).length > 0;
  const canEncrypt = keys.can_encrypt && !hasBcc;
  encryptBox.disabled = !canEncrypt;
  if (!canEncrypt) encryptBox.checked = false;

  // A reply to an encrypted message starts out encrypted, when it can be.
  if (!securityChosen && compose.replyTo !== null) {
    const was = securityOf.get(compose.replyTo);
    if (was?.encrypted && canEncrypt) encryptBox.checked = true;
    if (was?.signed && keys.can_sign) signBox.checked = true;
  }

  let hint = "";
  let warn = false;
  if (hasBcc && keys.can_encrypt) {
    hint = "Encrypted mail cannot have Bcc: each recipient's key is visible in it.";
  } else if (keys.missing.length && recipients.length) {
    hint = `No key for ${keys.missing.join(", ")} — import it in Settings → Encryption to encrypt.`;
  } else if (encryptBox.checked) {
    hint = "Only the recipients and you will be able to read it. The subject is not encrypted.";
  }
  if (encryptBox.checked && hasBcc) warn = true;
  securityHint.textContent = hint;
  securityHint.classList.toggle("warn", warn);
}

/// Each new draft starts undecided; called by `closeCompose`.
function resetComposeSecurity() {
  securityChosen = false;
  securityHint.textContent = "";
  composeSecurity.hidden = true;
}

// -- settings: keys -----------------------------------------------------------

const dateOnly = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short", year: "numeric" });

function groupFingerprint(fingerprint) {
  return fingerprint.replace(/(.{4})/g, "$1 ").trim();
}

function keysLayout() {
  keysPage.innerHTML = `
    <p class="lead">Sign mail so its recipients can tell it is really from you, and encrypt it so only they can read it. kuverta uses OpenPGP (PGP/MIME), which Thunderbird, GnuPG, Mailvelope and most other mail programs understand.</p>
    <section class="card">
      <h3>Your keys</h3>
      <div class="item-list" data-list="own" data-empty="You have no key yet. Create one below, or import the one you already use elsewhere."></div>
    </section>
    <section class="card">
      <h3>Other people's keys <span class="hint">what you encrypt to, and check signatures with</span></h3>
      <div class="item-list" data-list="theirs" data-empty="None yet. Import them below, or from a message that carries one."></div>
    </section>
    <section class="card">
      <h3>Create a key</h3>
      <form data-form="generate" autocomplete="off">
        <div class="grid2">
          <label>Name<input name="name" required placeholder="Erika Mustermann"></label>
          <label>Email address<select name="email"></select></label>
        </div>
        <div class="grid2">
          <label>Passphrase (optional)<input name="passphrase" type="password" autocomplete="new-password"></label>
          <label>Passphrase again<input name="again" type="password" autocomplete="new-password"></label>
        </div>
        <p class="hint">A modern Ed25519 key with an encryption subkey. The passphrase protects the key on disk and is kept in the system keychain; without one, kuverta makes one up and keeps that. Give the public half to the people who should write to you encrypted: “Copy public key” above.</p>
        <div class="card-actions"><button type="submit" class="primary">Create key</button></div>
      </form>
    </section>
    <section class="card">
      <h3>Import keys <span class="hint">public or secret, one or several</span></h3>
      <textarea data-el="armored" rows="5" spellcheck="false" placeholder="-----BEGIN PGP PUBLIC KEY BLOCK-----"></textarea>
      <div class="card-actions">
        <button type="button" data-el="import" class="primary">Import</button>
        <label class="file-button"><input type="file" data-el="file" accept=".asc,.gpg,.pgp,.txt,.key" hidden><span>Choose a file…</span></label>
      </div>
      <p class="hint" data-el="import-result"></p>
    </section>
  `;
  keysPage.querySelector("[data-el=import]").onclick = importKeys;
  keysPage.querySelector("[data-el=file]").addEventListener("change", async (event) => {
    const file = event.target.files?.[0];
    if (!file) return;
    keysPage.querySelector("[data-el=armored]").value = await file.text();
    event.target.value = "";
  });
  keysPage.querySelector("[data-form=generate]").addEventListener("submit", generateKey);
}

async function fillKeys() {
  if (!keysPage.firstElementChild) keysLayout();
  const emails = keysPage.querySelector("[name=email]");
  emails.textContent = "";
  for (const account of state.accounts) emails.append(new Option(account.email, account.email));
  if (state.email) emails.value = state.email;

  let keys = [];
  try {
    keys = await invoke("pgp_keys");
  } catch (err) {
    say(String(err), true);
  }
  const own = keysPage.querySelector("[data-list=own]");
  const theirs = keysPage.querySelector("[data-list=theirs]");
  own.textContent = "";
  theirs.textContent = "";
  for (const key of keys) (key.has_secret ? own : theirs).append(keyItem(key));
}

function keyItem(key) {
  const item = document.createElement("div");
  item.className = "item";
  const text = document.createElement("div");
  text.className = "grow";
  const title = document.createElement("div");
  title.className = "title";
  title.textContent = key.user_ids[0] ?? key.emails[0] ?? key.key_id;
  const sub = document.createElement("div");
  sub.className = "sub";
  sub.textContent = `${groupFingerprint(key.fingerprint)} · ${key.algorithm} · created ${dateOnly.format(new Date(key.created * 1000))}` +
    (key.expires ? ` · expires ${dateOnly.format(new Date(key.expires * 1000))}` : "");
  text.append(title, sub);
  if (key.user_ids.length > 1) {
    const more = document.createElement("div");
    more.className = "sub";
    more.textContent = `also ${key.user_ids.slice(1).join(", ")}`;
    text.append(more);
  }
  item.append(text);

  const chip = (label, kind = "") => {
    const span = document.createElement("span");
    span.className = `chip ${kind}`;
    span.textContent = label;
    item.append(span);
  };
  if (key.revoked) chip("revoked", "bad");
  else if (key.expired) chip("expired", "bad");
  else {
    if (key.can_sign && key.has_secret) chip("signs", "ok");
    if (key.can_encrypt) chip("encrypts", "ok");
  }
  if (key.has_secret && !key.has_passphrase) chip("needs passphrase", "bad");

  const button = (label, action, className = "") => {
    const b = document.createElement("button");
    b.type = "button";
    b.textContent = label;
    if (className) b.className = className;
    b.onclick = action;
    item.append(b);
  };
  button("Copy public key", async () => {
    try {
      const armored = await invoke("pgp_export_public", { fingerprint: key.fingerprint });
      await navigator.clipboard.writeText(armored);
      say("the public key is on the clipboard — paste it into a message or onto a key server");
    } catch (err) {
      say(String(err), true);
    }
  });
  if (key.has_secret && !key.has_passphrase) {
    button("Passphrase…", () => askPassphrase(key, item));
  }
  button("Delete", () => deleteKey(key), "danger");
  return item;
}

/// An inline field in the key's row, rather than a dialog: the passphrase is
/// checked against the key, then kept in the system keychain.
function askPassphrase(key, item) {
  if (item.querySelector("input[type=password]")) return;
  const field = document.createElement("input");
  field.type = "password";
  field.placeholder = "passphrase";
  field.autocomplete = "off";
  const save = document.createElement("button");
  save.type = "button";
  save.className = "primary";
  save.textContent = "Keep";
  const keep = async () => {
    try {
      await invoke("pgp_set_passphrase", { fingerprint: key.fingerprint, passphrase: field.value });
      say("passphrase kept");
      await fillKeys();
    } catch (err) {
      say(String(err), true);
      field.select();
    }
  };
  save.onclick = keep;
  field.addEventListener("keydown", (event) => {
    if (event.key === "Enter") keep();
  });
  item.append(field, save);
  field.focus();
}

async function deleteKey(key) {
  const who = key.user_ids[0] ?? key.fingerprint;
  const warning = key.has_secret
    ? `Delete your key ${who}? Mail encrypted to it can never be read again unless you have a copy elsewhere.`
    : `Delete the key of ${who}? You can import it again later.`;
  if (!confirm(warning)) return;
  try {
    await invoke("pgp_delete", { fingerprint: key.fingerprint });
    say("deleted");
    await fillKeys();
  } catch (err) {
    say(String(err), true);
  }
}

async function importKeys() {
  const field = keysPage.querySelector("[data-el=armored]");
  const result = keysPage.querySelector("[data-el=import-result]");
  const armored = field.value.trim();
  if (!armored) {
    result.textContent = "Paste a key, or choose a file.";
    return;
  }
  try {
    const report = await invoke("pgp_import", { armored });
    result.textContent = describeImport(report);
    if (report.imported.length) field.value = "";
    await fillKeys();
  } catch (err) {
    result.textContent = String(err);
  }
}

async function generateKey(event) {
  event.preventDefault();
  const form = event.target;
  const passphrase = form.passphrase.value;
  if (passphrase !== form.again.value) {
    say("the two passphrases are not the same", true);
    return;
  }
  const button = form.querySelector("[type=submit]");
  button.disabled = true;
  button.textContent = "Creating…";
  try {
    const key = await invoke("pgp_generate", {
      name: form.name.value.trim(),
      email: form.email.value,
      passphrase: passphrase || null,
    });
    say(`created a key for ${key.emails[0] ?? form.email.value}`);
    form.reset();
    await fillKeys();
  } catch (err) {
    say(String(err), true);
  } finally {
    button.disabled = false;
    button.textContent = "Create key";
  }
}

keysPage.addEventListener("show", fillKeys);
