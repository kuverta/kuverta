// Letters on paper, posted by LetterXpress: the settings under Settings →
// General, the sheet a PDF document is sent from, and the line its card
// shows once it has been.
//
// The settings say whether letters are printed at all. Until the person
// turns that on, a letter goes to the Postbox on the LetterXpress site and
// nothing is charged, so the first letter can be looked at before one is
// paid for. The API key goes one way, as a mail password does: the window
// sends it to the keychain and is told only whether one is there.
//
// The sheet asks LetterXpress what the letter costs before anything is
// sent, and again whenever an option changes; Send names the price.
// Nothing posts a letter but that button.

/// Money as the window's language writes it.
function money(amount, currency = "EUR") {
  try {
    return new Intl.NumberFormat(language(), { style: "currency", currency }).format(amount);
  } catch {
    return `${amount.toFixed(2)} ${currency}`;
  }
}

/// What a letter is, in one line: where it is, what it cost, and what
/// LetterXpress read as the address.
function letterSummary(letter) {
  const when = formatDate(letter.created_at);
  const test = letter.mode === "test";
  let what;
  switch (letter.state) {
    case "sending":
      what = t("Sent to LetterXpress on {when}, without an answer. It may have gone: look on the LetterXpress site before sending it again.", { when });
      break;
    case "failed":
      what = t("Not posted: {error}", { error: letter.last_error ?? "" });
      break;
    case "cancelled":
      what = t("Taken back on {when}.", { when: formatDate(letter.updated_at) });
      break;
    default:
      if (test) {
        what = t("In the LetterXpress Postbox since {when}: not printed until it is sent from there.", { when });
      } else if (letter.job_status === "done") {
        what = t("Posted on {when}.", { when });
      } else {
        what = t("With LetterXpress since {when}, to be printed and posted.", { when });
      }
  }
  const parts = [what];
  if (letter.cost != null && letter.state === "sent" && !test) parts.push(money(letter.cost));
  if (letter.tracking_status) parts.push(letter.tracking_status);
  else if (letter.tracking_code) parts.push(t("Tracking number {code}", { code: letter.tracking_code }));
  return parts.join("  ·  ");
}

/// A letter as a line on a document's card, with what can still be done
/// with it: ask where it is, or take it back while LetterXpress allows.
function letterLine(letter, onChange) {
  const line = document.createElement("div");
  line.className = "card-note letter-line";
  const text = document.createElement("span");
  text.textContent = letterSummary(letter);
  line.append(text);
  if (letter.address && letter.state !== "failed") {
    const address = document.createElement("div");
    address.className = "hint";
    address.textContent = t("Address as LetterXpress read it: {address}", { address: letter.address });
    line.append(address);
  }
  const replace = (next) => {
    const fresh = letterLine(next, onChange);
    line.replaceWith(fresh);
    onChange?.(next);
  };
  if (letter.job_id && letter.state === "sent") {
    cardButton(line, t("Where is it?"), async () => replace(await invoke("refresh_letter", { id: letter.id })));
  }
  if (letter.cancellable_until && letter.cancellable_until * 1000 > Date.now()) {
    cardButton(line, t("Take it back"), async (button) => {
      if (!(await ask(t("Take back {name}? LetterXpress allows this for 15 minutes after it was sent.", { name: letter.name }), { yes: t("Take it back"), danger: true }))) {
        button.disabled = false;
        return;
      }
      replace(await invoke("cancel_letter", { id: letter.id }));
    }, "danger");
  }
  return line;
}

// -- the sheet ---------------------------------------------------------------

const letterSheet = el("letter-sheet");
/// What the sheet is sending: the document, the latest quote, and who hears
/// about the letter once it went.
const posting = { doc: null, quote: null, sent: null, asked: 0, typing: null };

/// The address given by hand, or `null` when it is to be read off the page
/// as it stands.
function letterAddress() {
  const mode = el("letter-address-mode").value;
  if (mode === "page") return null;
  return {
    recipient: el("letter-recipient").value,
    sender: el("letter-sender").value.trim() || null,
    cover: mode === "cover",
  };
}

/// Shows the fields for an address given by hand, and says what will
/// happen to the page.
function showAddressFields() {
  const mode = el("letter-address-mode").value;
  el("letter-address-fields").hidden = mode === "page";
  el("letter-address-hint").textContent =
    mode === "cover"
      ? t("The address goes on a sheet of its own in front of the document, which costs a page more; printed on both sides, its back stays empty.")
      : mode === "over"
        ? t("The address is printed on white over the address field of the first page, covering what is there. Show the letter to check nothing else is under it.")
        : "";
}

function letterSpec() {
  const registered = el("letter-registered").value || null;
  return {
    color: el("letter-color").checked,
    duplex: el("letter-duplex").checked,
    international: el("letter-abroad").checked,
    registered,
  };
}

/// Asks for a price for the options as they are now. An answer to an
/// earlier question, overtaken by a change, is dropped.
async function quoteLetter() {
  const asked = ++posting.asked;
  const send = el("letter-send");
  send.disabled = true;
  posting.quote = null;
  el("letter-warning").hidden = true;
  const address = letterAddress();
  if (address && !address.recipient.trim()) {
    el("letter-price").textContent = t("Enter the address it goes to.");
    return;
  }
  el("letter-price").textContent = t("Asking LetterXpress for the price…");
  try {
    const quote = await invoke("quote_letter", {
      account: posting.doc.account_id,
      id: posting.doc.id,
      spec: letterSpec(),
      address,
    });
    if (asked !== posting.asked) return;
    posting.quote = quote;
    showQuote(quote);
    send.disabled = false;
  } catch (err) {
    if (asked !== posting.asked) return;
    el("letter-price").textContent = "";
    const warning = el("letter-warning");
    warning.textContent = String(err);
    warning.hidden = false;
  }
}

function showQuote(quote) {
  const test = quote.mode === "test";
  const pages = quote.pages === 1 ? t("one page") : t("{count} pages", { count: quote.pages });
  el("letter-price").textContent = test
    ? t("{pages} · {price} once it is posted", { pages, price: money(quote.price) })
    : t("{pages} · {price}", { pages, price: money(quote.price) });
  const balance = quote.balance
    ? " " + t("The LetterXpress account holds {amount}.", { amount: money(quote.balance.balance, quote.balance.currency) })
    : "";
  el("letter-mode").textContent = test
    ? t("Test mode: it goes to your Postbox on the LetterXpress site, to check and send from there. Nothing is printed or charged.") + balance
    : t("It is printed and posted with Deutsche Post. It can be taken back for 15 minutes, not after.") + balance;
  el("letter-send").textContent = test ? t("Put it in the Postbox") : t("Post it for {price}", { price: money(quote.price) });

  const warnings = [];
  const before = quote.sent_before.filter((letter) => letter.state === "sent" || letter.state === "sending");
  if (before.length) {
    warnings.push(
      before.length === 1
        ? t("This document went to LetterXpress before, on {when}.", { when: formatDate(before[0].created_at) })
        : t("This document went to LetterXpress {count} times before.", { count: before.length }),
    );
  }
  if (!test && quote.balance && quote.balance.balance < quote.price) {
    warnings.push(t("The account holds less than the letter costs; LetterXpress holds it back until it is topped up."));
  }
  const warning = el("letter-warning");
  warning.textContent = warnings.join(" ");
  warning.hidden = warnings.length === 0;
}

/// Opens the sheet for a PDF document. `onSent` hears the letter once
/// LetterXpress took it.
async function openLetterSheet(doc, onSent) {
  posting.doc = doc;
  posting.sent = onSent ?? null;
  el("letter-title").textContent = t("Send {name} by post", { name: doc.name });
  el("letter-sub").textContent = t("LetterXpress reads the address from where the envelope window sits on the first page: 20 mm from the left, 45 mm from the top. If it is not there, enter it here.");
  el("letter-address-mode").value = "page";
  el("letter-recipient").value = "";
  el("letter-sender").value = "";
  showAddressFields();
  invoke("letter_settings")
    .then((settings) => {
      if (posting.doc === doc && !el("letter-sender").value) el("letter-sender").value = settings.sender ?? "";
    })
    .catch(() => {});
  el("letter-color").checked = false;
  el("letter-duplex").checked = false;
  el("letter-abroad").checked = false;
  el("letter-registered").value = "";
  el("letter-registered").disabled = false;
  el("letter-mode").textContent = "";
  el("letter-send").textContent = t("Send");
  letterSheet.hidden = false;
  el("letter-cancel").focus();
  await quoteLetter();
}

function closeLetterSheet() {
  letterSheet.hidden = true;
  clearTimeout(posting.typing);
  posting.doc = null;
  posting.quote = null;
  posting.sent = null;
  posting.asked++;
}

for (const id of ["letter-color", "letter-duplex", "letter-registered"]) {
  el(id).addEventListener("change", () => quoteLetter());
}
// Registered mail goes only within Germany.
el("letter-abroad").addEventListener("change", (event) => {
  const registered = el("letter-registered");
  if (event.target.checked) registered.value = "";
  registered.disabled = event.target.checked;
  quoteLetter();
});

el("letter-address-mode").addEventListener("change", () => {
  showAddressFields();
  if (el("letter-address-mode").value !== "page") el("letter-recipient").focus();
  quoteLetter();
});
// Typing is asked about once it pauses, not at every key.
for (const id of ["letter-recipient", "letter-sender"]) {
  el(id).addEventListener("input", () => {
    clearTimeout(posting.typing);
    el("letter-send").disabled = true;
    posting.typing = setTimeout(quoteLetter, 500);
  });
}

/// The letter as it will be printed, in the viewer. The sheet steps aside
/// while it is shown and comes back when the viewer closes.
el("letter-show").onclick = async () => {
  const doc = posting.doc;
  if (!doc) return;
  try {
    const file = await invoke("letter_pdf", {
      account: doc.account_id,
      id: doc.id,
      spec: letterSpec(),
      address: letterAddress(),
    });
    letterSheet.hidden = true;
    el("attachment-sheet").addEventListener(
      "close",
      () => {
        if (posting.doc === doc) letterSheet.hidden = false;
      },
      { once: true },
    );
    await openOutgoing({ ...file, size: Math.floor((file.data.length * 3) / 4) });
  } catch (err) {
    const warning = el("letter-warning");
    warning.textContent = String(err);
    warning.hidden = false;
  }
};

el("letter-cancel").onclick = closeLetterSheet;
letterSheet.addEventListener("close", closeLetterSheet);

el("letter-send").onclick = async () => {
  const { doc, quote } = posting;
  if (!doc || !quote) return;
  const send = el("letter-send");
  send.disabled = true;
  const onSent = posting.sent;
  try {
    const letter = await invoke("send_letter", {
      account: doc.account_id,
      id: doc.id,
      spec: letterSpec(),
      address: letterAddress(),
      quoted: quote.price,
    });
    closeLetterSheet();
    say(
      letter.mode === "test"
        ? t("{name} is in the LetterXpress Postbox", { name: doc.name })
        : t("{name} is with LetterXpress, to be printed and posted", { name: doc.name }),
    );
    onSent?.(letter);
  } catch (err) {
    send.disabled = false;
    const warning = el("letter-warning");
    warning.textContent = String(err);
    warning.hidden = false;
  }
};

// -- Settings → General → Letters by post --------------------------------------

{
  const page = SETTINGS_PAGES.general;
  const status = el("letter-status");
  let settings = null;

  function render(next) {
    settings = next;
    el("letter-username").value = next.username ?? "";
    el("letter-key").value = "";
    el("letter-key").placeholder = next.has_key ? t("kept in the keychain") : "";
    el("letter-key-status").textContent = next.has_key
      ? t("An API key is kept in the keychain. Enter a new one to replace it.")
      : t("No API key yet.");
    el("letter-forget").hidden = !next.has_key;
    el("letter-live").checked = next.mode === "live";
  }

  async function show() {
    status.textContent = "";
    try {
      render(await invoke("letter_settings"));
    } catch (err) {
      status.textContent = String(err);
    }
  }

  async function save() {
    const live = el("letter-live").checked;
    if (live && settings?.mode !== "live") {
      const yes = await ask(
        t("Print and post letters from now on? LetterXpress charges for every letter sent, and after 15 minutes one cannot be taken back."),
        { yes: t("Print and post") },
      );
      if (!yes) {
        el("letter-live").checked = false;
        return false;
      }
    }
    const key = el("letter-key").value.trim();
    if (key) await invoke("set_letter_key", { key });
    render(
      await invoke("save_letter_settings", {
        input: { username: el("letter-username").value.trim(), mode: live ? "live" : "test" },
      }),
    );
    return true;
  }

  page.addEventListener("show", show);

  el("letter-save").onclick = async () => {
    try {
      if (await save()) say(t("letter settings kept"));
    } catch (err) {
      status.textContent = String(err);
    }
  };

  el("letter-check").onclick = async () => {
    const button = el("letter-check");
    button.disabled = true;
    status.textContent = t("Asking LetterXpress…");
    try {
      if (!(await save())) return;
      const balance = await invoke("check_letter_account");
      status.textContent = t("LetterXpress knows the account. It holds {amount}.", {
        amount: money(balance.balance, balance.currency),
      });
    } catch (err) {
      status.textContent = String(err);
    } finally {
      button.disabled = false;
    }
  };

  el("letter-forget").onclick = async () => {
    if (!(await ask(t("Remove the LetterXpress API key from the keychain?"), { yes: t("Remove"), danger: true }))) return;
    try {
      await invoke("clear_letter_key");
      await show();
    } catch (err) {
      status.textContent = String(err);
    }
  };
}
