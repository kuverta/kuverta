// Mail as the sender designed it.
//
// A message with HTML gets two tabs over its body: Text, which is what the
// pane has always shown, and HTML, which is the sender's page in a frame
// that can do nothing. The frame is `sandbox` with no flags and loads from
// the app's own scheme (see src/page.rs), where the page is made from an
// allow-list of what mail needs and nothing it does not: no scripts, no
// forms, nothing fetched from anywhere but the app itself.
//
// Two things in a page can reach out, and both are asked about here:
//
// - **Pictures from the internet** are not in the page until the person says
//   so for this message. The app says how many were left out, and why that
//   matters — showing them tells the sender the message was opened — and
//   the choice lasts as long as the message is open.
// - **Links** never go where they say. A click arrives as an event with the
//   real address and what the link said, and a sheet shows both before
//   anything is opened: an unencrypted page, an address with letters that
//   look like other letters, a link whose words name one site and whose
//   address names another, are each said in a sentence.
//
// The choice of tab is kept across messages and restarts, like Post's.

const htmlFrame = el("reading-html");
const mailTabs = el("mail-tabs");
const remoteBar = el("html-remote");
const linkSheet = el("link-sheet");

/// Which tab a message with HTML opens on. Text by default: it is what the
/// pane has always shown, and it is the one that cannot be designed to
/// mislead.
let mailView = (() => {
  try {
    return localStorage.getItem("mailView") === "html" ? "html" : "text";
  } catch {
    return "text";
  }
})();

/// The message the frame is about, and what has been allowed for it.
const mailPage = { account: null, id: null, hasHtml: false, remote: false, loading: 0 };

/// Called with the open message. Shows the tabs when there is a page to
/// show, and the page when that is the tab chosen.
function showMailView(detail, account) {
  mailPage.account = account;
  mailPage.id = detail.id;
  mailPage.hasHtml = Boolean(detail.has_html);
  // Pictures are a choice per message: a new message starts without them.
  mailPage.remote = false;
  remoteBar.hidden = true;
  mailTabs.hidden = !mailPage.hasHtml;
  renderMailView();
}

/// Post, or nothing open: no tabs, no frame.
function hideMailView() {
  mailPage.account = null;
  mailPage.id = null;
  mailPage.hasHtml = false;
  mailTabs.hidden = true;
  remoteBar.hidden = true;
  htmlFrame.hidden = true;
  unloadFrame();
}

function setMailView(view) {
  mailView = view;
  try {
    localStorage.setItem("mailView", view);
  } catch {
    // A preference, not state: without storage it lasts until the window closes.
  }
  renderMailView();
}

function renderMailView() {
  const html = mailPage.hasHtml && mailView === "html";
  for (const tab of mailTabs.querySelectorAll("[data-mail]")) {
    const active = tab.dataset.mail === (html ? "html" : "text");
    tab.classList.toggle("active", active);
    tab.setAttribute("aria-selected", String(active));
  }
  el("reading-body").hidden = html;
  htmlFrame.hidden = !html;
  if (html) loadFrame();
  else {
    remoteBar.hidden = true;
    unloadFrame();
  }
}

/// Nothing in the frame: the last message's page does not stay in memory
/// behind the next message's text.
function unloadFrame() {
  mailPage.loading += 1;
  if (htmlFrame.getAttribute("src")) htmlFrame.src = "about:blank";
}

/// Points the frame at the message's page. The address comes from the app,
/// which knows how the scheme is spelled on this platform.
async function loadFrame() {
  const { account, id, remote } = mailPage;
  const generation = (mailPage.loading += 1);
  let url;
  try {
    url = await invoke("mail_page_url", { account, id, remote });
  } catch (err) {
    say(t("could not show the mailPage: {error}", { error: err }), true);
    return;
  }
  // The person moved on while the address was fetched.
  if (generation !== mailPage.loading || mailPage.id !== id) return;
  htmlFrame.src = url;
}

for (const tab of mailTabs.querySelectorAll("[data-mail]")) {
  tab.onclick = () => setMailView(tab.dataset.mail);
}

/// A page was made, and it says how many pictures it left out.
function pageMade(made) {
  if (made.id !== mailPage.id || made.remote !== mailPage.remote) return;
  const note = el("html-remote-note");
  const show = el("html-remote-show");
  if (made.remote) {
    note.textContent = t("Pictures from the internet are shown for this message.");
    show.hidden = true;
    remoteBar.hidden = false;
    return;
  }
  if (!made.remote_blocked) {
    remoteBar.hidden = true;
    return;
  }
  note.textContent =
    made.remote_blocked === 1
      ? t("One picture from the internet is not shown. Showing it tells the sender this message was opened, and when.")
      : t("{count} pictures from the internet are not shown. Showing them tells the sender this message was opened, and when.", {
          count: made.remote_blocked,
        });
  show.hidden = false;
  remoteBar.hidden = false;
}

el("html-remote-show").onclick = () => {
  mailPage.remote = true;
  remoteBar.hidden = true;
  loadFrame();
};

// -- links -------------------------------------------------------------------

/// The link being asked about.
let askedLink = null;

/// What is odd about an address, each in a sentence — the things a link in
/// a mail is made to hide. The sentences are the warning; the address is
/// shown whole in any case.
function linkWarnings(url, said) {
  const warnings = [];
  let parsed = null;
  try {
    parsed = new URL(url);
  } catch {
    warnings.push(t("The address could not be read as one."));
    return warnings;
  }
  if (parsed.protocol === "http:") {
    warnings.push(t("Not encrypted: the mailPage, and anything typed on it, can be read on the way."));
  }
  const host = parsed.hostname.toLowerCase();
  if (host.split(".").some((label) => label.startsWith("xn--"))) {
    warnings.push(t("The site's name uses letters that can look like other letters: {host}.", { host }));
  }
  if (parsed.username || parsed.password) {
    warnings.push(t("The part before the @ is not the site. It goes to {host}.", { host }));
  }
  if (/^\d+\.\d+\.\d+\.\d+$/.test(host) || host.startsWith("[")) {
    warnings.push(t("The site is a bare number, not a name."));
  }
  // The link's words name a site, and the address names another.
  const saidHost = hostNamedIn(said);
  if (saidHost && !sameSite(saidHost, host)) {
    warnings.push(t("The link says {said} but goes to {host}.", { said: saidHost, host }));
  }
  return warnings;
}

/// The site a link's words name, when they look like an address.
function hostNamedIn(text) {
  const match = /(?:^|\s)(?:https?:\/\/)?(?:www\.)?([a-z0-9-]+(?:\.[a-z0-9-]+)+)(?:[/?#:]|\s|$)/i.exec(text ?? "");
  return match ? match[1].toLowerCase() : null;
}

/// Whether two names are the same site: one is the other, or a name under
/// it. `shop.example.de` is `example.de`; `example.de.evil.example` is not.
function sameSite(said, host) {
  const strip = (name) => name.replace(/^www\./, "");
  const a = strip(said);
  const b = strip(host);
  return a === b || b.endsWith(`.${a}`) || a.endsWith(`.${b}`);
}

/// A `mailto:` address, as compose takes it.
function mailtoDraft(url) {
  const rest = url.slice("mailto:".length);
  const [addresses, query = ""] = rest.split("?");
  const draft = { to: decodeURIComponent(addresses).split(",").map((a) => a.trim()).filter(Boolean) };
  for (const pair of query.split("&")) {
    const [name, value = ""] = pair.split("=");
    const decoded = decodeURIComponent(value.replace(/\+/g, " "));
    if (name.toLowerCase() === "subject") draft.subject = decoded;
    if (name.toLowerCase() === "body") draft.body = decoded;
    if (name.toLowerCase() === "cc") draft.cc = decoded.split(",").map((a) => a.trim()).filter(Boolean);
  }
  return draft;
}

/// A link in the page was clicked: asks before anything is opened.
function askAboutLink(link) {
  const url = String(link.url ?? "");
  const said = String(link.text ?? "").trim();
  const lower = url.toLowerCase();
  const kind = lower.startsWith("mailto:") ? "mail" : lower.startsWith("tel:") ? "phone" : "web";
  askedLink = { url, said, kind };

  const title = el("link-title");
  const sub = el("link-sub");
  const go = el("link-go");
  const copy = el("link-copy");
  const saidLine = el("link-said");
  const warnings = el("link-warnings");

  el("link-address").textContent = url;
  saidLine.hidden = !said || said === url;
  saidLine.textContent = said ? t("The link says: “{said}”", { said }) : "";
  warnings.textContent = "";
  warnings.hidden = true;

  if (kind === "mail") {
    const draft = mailtoDraft(url);
    title.textContent = t("Write to {address}?", { address: draft.to.join(", ") || url });
    sub.textContent = t("A mail address in the message. Nothing is sent until you send it.");
    go.textContent = t("Write to them");
    go.hidden = false;
    copy.hidden = false;
  } else if (kind === "phone") {
    title.textContent = t("A phone number");
    sub.textContent = t("The message links to a number. kuverta does not place calls; copy it if you want it.");
    go.hidden = true;
    copy.hidden = false;
  } else {
    title.textContent = t("Open this link?");
    sub.textContent = t("It was in the message. kuverta has not looked at where it goes.");
    go.textContent = t("Open in the browser");
    go.hidden = false;
    copy.hidden = false;
    for (const warning of linkWarnings(url, said)) {
      const item = document.createElement("li");
      item.textContent = warning;
      warnings.append(item);
    }
    warnings.hidden = !warnings.childElementCount;
  }
  linkSheet.hidden = false;
  // The harmless answer has the focus: an Enter meant for something else
  // must not open a page.
  el("link-cancel").focus();
}

el("link-cancel").onclick = () => closeDialog(linkSheet);

el("link-copy").onclick = async () => {
  if (!askedLink) return;
  const address = askedLink.kind === "phone" ? askedLink.url.slice("tel:".length) : askedLink.url;
  try {
    await navigator.clipboard.writeText(address);
    say(t("Address copied."));
  } catch {
    // Without the clipboard, the address is still on screen to select.
    const range = document.createRange();
    range.selectNodeContents(el("link-address"));
    const selection = window.getSelection();
    selection.removeAllRanges();
    selection.addRange(range);
    say(t("Could not copy — the address is selected; press ⌘C."));
  }
};

el("link-go").onclick = async () => {
  const link = askedLink;
  closeDialog(linkSheet);
  if (!link) return;
  if (link.kind === "mail") {
    await openComposeWith(mailtoDraft(link.url), { what: t("New message") });
    return;
  }
  if (link.kind !== "web") return;
  try {
    await invoke("open_external", { url: link.url });
  } catch (err) {
    say(String(err), true);
  }
};

linkSheet.addEventListener("close", () => {
  askedLink = null;
});

// The page's own words arrive from the app: how many pictures it left out,
// and the links that were clicked. Listening is a plugin command, which
// Tauri checks against capabilities/main.json; a refusal is said, not
// swallowed, because a window that cannot hear shows a page whose links
// do nothing.
for (const [name, heard] of [
  ["mail-page", (event) => pageMade(event.payload ?? {})],
  ["mail-link", (event) => askAboutLink(event.payload ?? {})],
]) {
  window.__TAURI__?.event?.listen(name, heard).catch((err) => say(String(err), true));
}
