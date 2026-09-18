// After a delete: "these look like the one you just deleted".
//
// Spam and bulk mail come in runs — the same sender with a new order number
// in the subject, the same list, or the same text from a rotating cast of
// senders. The core finds the run; this shows it, every message ticked and
// listed so nothing goes that was not seen going, and offers the two useful
// answers: delete them all now, or gather them in a smart mailbox — prefilled
// with rules that would catch the next one — to deal with later.

const similarSheet = el("similar-sheet");
const similarList = el("similar-list");

/// The report on screen, and the account it is about.
let similarReport = null;
let similarAccount = null;

function askAboutSimilar() {
  try {
    return localStorage.getItem("askSimilar") !== "no";
  } catch {
    return true;
  }
}

function setAskAboutSimilar(ask) {
  try {
    localStorage.setItem("askSimilar", ask ? "yes" : "no");
  } catch {
    // A preference: without storage it lasts until the window closes.
  }
}

/// Called with the row just moved to the Trash.
async function offerSimilar(row) {
  if (!askAboutSimilar() || state.postbox || state.account === null) return;
  const account = state.account;
  let report;
  try {
    report = await invoke("similar", { account, id: row.id });
  } catch (err) {
    // Not worth an error: the delete itself worked.
    invoke("log_ui", { level: "warn", message: `similar messages: ${err}` }).catch(() => {});
    return;
  }
  if (!report.matches.length || state.account !== account) return;
  similarReport = report;
  similarAccount = account;

  const count = report.matches.length;
  el("similar-title").textContent = `${count} more like “${row.subject}”`;
  const reasons = [...new Set(report.matches.map((m) => m.reason))];
  el("similar-sub").textContent = `From ${row.from}. They look alike because of: ${reasons.join(", ")}. Untick any to keep.`;

  similarList.textContent = "";
  for (const match of report.matches) {
    const pick = document.createElement("label");
    pick.className = "pick";
    const tick = document.createElement("input");
    tick.type = "checkbox";
    tick.checked = true;
    tick.value = String(match.id);
    tick.addEventListener("change", syncSimilarCount);
    const who = document.createElement("span");
    who.className = "who";
    who.textContent = match.from;
    const when = document.createElement("span");
    when.className = "when";
    when.textContent = listDate(match.date_utc);
    const what = document.createElement("span");
    what.className = "what";
    what.textContent = match.subject;
    const why = document.createElement("span");
    why.className = "why";
    why.textContent = match.reason;
    pick.append(tick, who, when, what, why);
    similarList.append(pick);
  }
  el("similar-all").checked = true;
  el("similar-never").checked = false;
  syncSimilarCount();
  similarSheet.hidden = false;
  // The harmless answer has the focus. This opens a moment after a delete,
  // and an Enter meant for the list must not delete the rest.
  el("similar-keep").focus();
}

function chosenSimilar() {
  return [...similarList.querySelectorAll("input:checked")].map((tick) => Number(tick.value));
}

function syncSimilarCount() {
  const chosen = chosenSimilar().length;
  const button = el("similar-trash");
  button.textContent = chosen ? `Move ${chosen} to Trash` : "Move to Trash";
  button.disabled = chosen === 0;
  el("similar-all").checked = chosen === similarReport.matches.length;
  el("similar-all").indeterminate = chosen > 0 && chosen < similarReport.matches.length;
}

el("similar-all").addEventListener("change", (event) => {
  for (const tick of similarList.querySelectorAll("input")) tick.checked = event.target.checked;
  syncSimilarCount();
});

el("similar-trash").onclick = async () => {
  const ids = chosenSimilar();
  const account = similarAccount;
  closeDialog(similarSheet);
  if (!state.trash || account !== state.account) return;
  let moved = 0;
  let failed = null;
  // One at a time, like any batch: each is its own queued move with its own
  // undo, and a refusal on the fifth keeps the first four.
  for (const id of ids) {
    try {
      await invoke("move_to", { account, id, target: state.trash });
      moved += 1;
    } catch (err) {
      failed = err;
    }
  }
  say(
    failed
      ? `moved ${moved} to ${state.trash}, then: ${failed}`
      : `moved ${moved} more to ${state.trash} — z undoes them one at a time`,
    Boolean(failed),
  );
  await reload({ keepPosition: true });
};

el("similar-smart").onclick = () => {
  const suggestion = similarReport?.suggestion;
  closeDialog(similarSheet);
  if (suggestion) openSmartEditor({ name: suggestion.name, query: suggestion.query });
};

el("similar-keep").onclick = () => closeDialog(similarSheet);

similarSheet.addEventListener("close", () => {
  if (el("similar-never").checked) {
    setAskAboutSimilar(false);
    say("won't ask again — it can be turned back on in Settings → General");
  }
});

// The same preference, in Settings → General.
{
  const pref = el("pref-similar");
  SETTINGS_PAGES.general.addEventListener("show", () => (pref.checked = askAboutSimilar()));
  pref.addEventListener("change", () => setAskAboutSimilar(pref.checked));
}
