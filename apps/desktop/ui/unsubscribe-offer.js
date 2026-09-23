// After a delete: "this one came from a list — shall I take you off it?"
//
// Deleting a newsletter is the moment somebody has decided they do not want
// it. The Cleanup page can unsubscribe from everything at once, but it has to
// be gone to; here the offer is made where the decision was taken, for the one
// sender, with what it will do said plainly — one click, a mail sent, or a
// page to open.
//
// The offer is only made when there is something to act on: the message
// carries a usable List-Unsubscribe, and this sender has not been
// unsubscribed from before.

const unsubSheet = el("unsub-sheet");

/// What the sheet is about: the sender view and the account it belongs to.
let unsubSender = null;
let unsubAccount = null;

function askAboutUnsubscribe() {
  try {
    return localStorage.getItem("askUnsubscribe") !== "no";
  } catch {
    return true;
  }
}

function setAskAboutUnsubscribe(ask) {
  try {
    localStorage.setItem("askUnsubscribe", ask ? "yes" : "no");
  } catch {
    // A preference: without storage it lasts until the window closes.
  }
}

/// What unsubscribing from this sender will actually do, in one line.
function unsubscribeWay(method) {
  if (method.kind === "one_click") return t("kuverta tells them once, and that is all.");
  if (method.kind === "mailto") return t("kuverta sends them a mail asking to be taken off.");
  return t("It opens their page in the browser; the last step is yours.");
}

/// Called with the row just moved to the Trash. Answers whether it took the
/// offer, so the look-alikes are not asked about on top of it.
async function offerUnsubscribe(row) {
  if (!askAboutUnsubscribe() || state.postbox || state.account === null) return false;
  const account = state.account;
  let sender;
  try {
    sender = await invoke("unsubscribe_for_message", { account, id: row.id });
  } catch (err) {
    // Not worth an error: the delete itself worked.
    invoke("log_ui", { level: "warn", message: `unsubscribe offer: ${err}` }).catch(() => {});
    return false;
  }
  if (!sender || state.account !== account) return false;
  unsubSender = sender;
  unsubAccount = account;

  el("unsub-title").textContent = t("Stop mail from {sender}?", { sender: sender.name });
  el("unsub-sub").textContent = t("That one was a newsletter. {way}", { way: unsubscribeWay(sender.method) });
  const rest = sender.messages - 1;
  el("unsub-also").parentElement.hidden = rest < 1;
  el("unsub-also").checked = false;
  el("unsub-also-label").textContent =
    rest === 1
      ? t("Move the one other message from them to Trash too")
      : t("Move their other {count} messages to Trash too", { count: rest });
  el("unsub-never").checked = false;
  el("unsub-go").textContent =
    sender.method.kind === "browser" ? t("Open their page") : t("Unsubscribe");
  unsubSheet.hidden = false;
  // The harmless answer has the focus: this opens a moment after a delete, and
  // an Enter meant for the list must not send anything to anybody.
  el("unsub-keep").focus();
  return true;
}

el("unsub-go").onclick = async () => {
  const sender = unsubSender;
  const account = unsubAccount;
  const alsoTrash = el("unsub-also").checked;
  closeDialog(unsubSheet);
  if (!sender || account !== state.account) return;
  const email = state.accounts.find((one) => one.id === account)?.email;
  if (!email) return;

  say(t("Unsubscribing from {sender}…", { sender: sender.name }));
  let results;
  try {
    results = await invoke("unsubscribe", { email, keys: [sender.key] });
  } catch (err) {
    say(String(err), true);
    return;
  }
  const result = results[0];
  if (!result) return;
  if (result.state === "open" && result.url) {
    // As far as this can follow it: their page wants a person, so it is
    // opened and the attempt written down.
    await invoke("open_external", { url: result.url }).catch((err) => say(String(err), true));
    await invoke("unsubscribe_opened", { account, key: sender.key, url: result.url }).catch(() => {});
    say(t("Their page is open in the browser."));
  } else if (result.state === "done") {
    say(t("Unsubscribed from {sender}.", { sender: sender.name }));
  } else {
    say(t("{sender} could not be unsubscribed from: {detail}", {
      sender: sender.name,
      detail: result.detail ?? t("no reason given"),
    }), true);
    return;
  }

  if (alsoTrash) {
    try {
      const moved = await invoke("trash_from_sender", { account, key: sender.key });
      say(t("{count} more moved to Trash.", { count: moved }));
      await reload({ keepPosition: true });
    } catch (err) {
      say(String(err), true);
    }
  }
  refreshCleanupCount().catch(() => {});
};

el("unsub-keep").onclick = () => closeDialog(unsubSheet);

unsubSheet.addEventListener("close", () => {
  if (el("unsub-never").checked) {
    setAskAboutUnsubscribe(false);
    say(t("won't ask again — it can be turned back on in Settings → General"));
  }
});

// The same preference, in Settings → General.
{
  const pref = el("pref-unsubscribe");
  SETTINGS_PAGES.general.addEventListener("show", () => (pref.checked = askAboutUnsubscribe()));
  pref.addEventListener("change", () => setAskAboutUnsubscribe(pref.checked));
}
