// Attachments: what a message carries besides its text.
//
// The open message lists them under its header, one button each. A button
// opens the viewer, which shows what the window can show itself — an image,
// a PDF, plain text — and offers Save to Downloads and Open in another app
// for everything. Pictures a message shows inside its text (logos, mostly)
// are listed only on request.
//
// Opening in another app is refused for anything that could run something —
// a program, a script, a web page — and the button says so rather than
// failing: those can be saved, and opening them is a step the person takes
// themselves, knowing where the file came from.
//
// Safe preview, on unless turned off in Settings → General, draws pictures
// and PDFs without the system's own readers: kuverta's worker process draws
// them, locked down, and hands back pages as pictures it made itself; Word,
// Excel and web pages come back as their text. See core-rpc's preview.rs.
// Opening a file in another app then leaves that protection, so it takes a
// second, deliberate click.
//
// The same viewer shows the documents the assistant makes — a letter it
// wrote, a PDF it signed — which have the same four commands under other
// names (see `viewerCall`), and are drawn in safe preview too: a signed copy
// of a stranger's PDF is mostly the stranger's PDF.

const attachmentSheet = el("attachment-sheet");
const viewer = { account: null, messageId: null, attachment: null, url: null, generation: 0, openArmed: false };

/// The command that fetches, draws, saves or opens what the viewer shows:
/// an attachment of a message, or — when `attachment.document` is set — a
/// document the assistant made, which has the same four.
function viewerCall(what, extra = {}) {
  const { account, messageId, attachment } = viewer;
  if (attachment.document != null) {
    const names = { attachment: "document", attachment_preview: "document_preview", save_attachment: "save_document", open_attachment: "open_document" };
    return invoke(names[what], { id: attachment.document, ...extra });
  }
  return invoke(what, { account, id: messageId, index: attachment.index, ...extra });
}

/// A document the assistant made, as the viewer takes it.
function documentAsAttachment(doc) {
  return {
    index: 0,
    name: doc.name,
    content_type: doc.content_type,
    size: doc.size,
    inline: false,
    preview: "pdf",
    risky: false,
    document: doc.id,
  };
}

/// Opens the viewer on a document the assistant made.
function openDocument(doc) {
  return openAttachment(null, null, documentAsAttachment(doc));
}

/// A chip for a document, like an attachment's, that opens it in the viewer.
function documentChip(doc) {
  const chip = document.createElement("button");
  chip.type = "button";
  chip.className = "attachment-chip";
  chip.title = doc.signed ? t("{name} — signed", { name: doc.name }) : doc.name;
  const icon = document.createElement("span");
  icon.className = "icon";
  icon.textContent = doc.signed ? "✍️" : "📄";
  const name = document.createElement("span");
  name.className = "name";
  name.textContent = doc.name;
  const size = document.createElement("span");
  size.className = "size";
  size.textContent = sizeText(doc.size);
  chip.append(icon, name, size);
  chip.onclick = () => openDocument(doc);
  return chip;
}

/// Pages safe preview asks for at a time: a first look fast, more on request.
const SAFE_PAGES_AT_ONCE = 2;

/// Whether attachments are drawn by safe preview. On unless turned off.
function safePreview() {
  try {
    return localStorage.getItem("safePreview") !== "no";
  } catch {
    return true;
  }
}

function sizeText(bytes) {
  if (bytes < 1024) return t("{count} bytes", { count: bytes });
  if (bytes < 1024 * 1024) return t("{count} KB", { count: Math.round(bytes / 1024) });
  return t("{count} MB", { count: (bytes / 1024 / 1024).toFixed(1) });
}

function attachmentIcon(attachment) {
  const type = attachment.content_type;
  if (attachment.preview === "image") return "🖼";
  if (attachment.preview === "pdf") return "📄";
  if (type === "message/rfc822") return "✉️";
  if (type === "text/calendar") return "📅";
  if (attachment.preview === "text") return "📝";
  if (/zip|compressed|x-7z|x-rar|gzip|x-tar/.test(type)) return "🗜";
  return "📎";
}

/// A button for one attachment — icon, name, size — that opens the viewer.
function attachmentChip(account, messageId, attachment) {
  const chip = document.createElement("button");
  chip.type = "button";
  chip.className = `attachment-chip${attachment.risky ? " risky" : ""}`;
  chip.title = attachment.risky
    ? t("{name} — could run something when opened", { name: attachment.name })
    : t("{name} — {type}", { name: attachment.name, type: attachment.content_type });
  const icon = document.createElement("span");
  icon.className = "icon";
  icon.textContent = attachmentIcon(attachment);
  const name = document.createElement("span");
  name.className = "name";
  // textContent: the name is the sender's.
  name.textContent = attachment.name;
  const size = document.createElement("span");
  size.className = "size";
  size.textContent = sizeText(attachment.size);
  chip.append(icon, name, size);
  chip.onclick = () => openAttachment(account, messageId, attachment);
  return chip;
}

/// Fills the reading pane's strip for an opened message.
function showAttachments(detail, account = state.account) {
  const strip = el("reading-attachments");
  strip.textContent = "";
  const all = detail.attachments ?? [];
  const own = all.filter((a) => !a.inline);
  const inline = all.filter((a) => a.inline);
  if (!all.length) {
    strip.hidden = true;
    return;
  }
  for (const attachment of own) strip.append(attachmentChip(account, detail.id, attachment));
  if (inline.length) {
    const more = document.createElement("button");
    more.type = "button";
    more.className = "link-button attachment-more";
    more.textContent =
      inline.length === 1 ? t("one picture in the text") : t("{count} pictures in the text", { count: inline.length });
    more.onclick = () => {
      more.replaceWith(...inline.map((a) => attachmentChip(account, detail.id, a)));
    };
    strip.append(more);
  }
  strip.hidden = false;
}

function hideAttachments() {
  const strip = el("reading-attachments");
  strip.textContent = "";
  strip.hidden = true;
}

function clearViewer() {
  if (viewer.url) URL.revokeObjectURL(viewer.url);
  viewer.url = null;
  for (const id of ["attachment-image", "attachment-pdf"]) {
    el(id).removeAttribute("src");
    el(id).hidden = true;
  }
  el("attachment-text").textContent = "";
  el("attachment-text").hidden = true;
  const pages = el("attachment-pages");
  for (const page of pages.querySelectorAll("img")) page.remove();
  pages.hidden = true;
  el("attachment-more").hidden = true;
  el("attachment-safe").hidden = true;
}

/// Shows what the worker drew or read, from `first` on.
async function showSafePreview(generation, first) {
  const status = el("attachment-status");
  const more = el("attachment-more");
  more.disabled = true;
  const shown = await viewerCall("attachment_preview", {
    first,
    count: first === 0 ? SAFE_PAGES_AT_ONCE : 4,
  });
  if (generation !== viewer.generation || attachmentSheet.hidden) return;
  more.disabled = false;
  if (shown.kind === "pages") {
    const pages = el("attachment-pages");
    shown.pages.forEach((png, at) => {
      const page = document.createElement("img");
      // A PNG kuverta wrote from the pixels the worker drew: never the file.
      page.src = `data:image/png;base64,${png}`;
      page.alt = shown.total > 1 ? t("Page {page}", { page: shown.first + at + 1 }) : "";
      pages.insertBefore(page, more);
    });
    pages.hidden = false;
    const next = shown.first + shown.pages.length;
    more.hidden = next >= shown.total;
    more.textContent = t("More pages ({shown} of {total})", { shown: next, total: shown.total });
    more.onclick = () =>
      showSafePreview(generation, next).catch((err) => {
        status.textContent = t("could not load it: {error}", { error: err });
      });
    status.textContent = "";
  } else if (shown.kind === "text") {
    const text = el("attachment-text");
    text.textContent = shown.text;
    text.hidden = false;
    status.textContent = "";
  } else {
    status.textContent = shown.why;
  }
  el("attachment-safe").hidden = false;
}

/// Opens the viewer on one attachment and loads it when it can be shown.
async function openAttachment(account, messageId, attachment) {
  const generation = ++viewer.generation;
  Object.assign(viewer, { account, messageId, attachment });
  clearViewer();
  el("attachment-title").textContent = attachment.name;
  el("attachment-sub").textContent = [
    attachment.content_type,
    sizeText(attachment.size),
    attachment.risky ? t("could run something when opened: save it only if you trust the sender") : "",
  ]
    .filter(Boolean)
    .join("  ·  ");
  const open = el("attachment-open");
  open.disabled = attachment.risky;
  open.title = attachment.risky ? t("Not opened from here: it could run something. Save it instead.") : "";
  viewer.openArmed = false;
  open.textContent = t("Open in another app");
  open.classList.remove("danger");
  const status = el("attachment-status");
  // Plain text is shown as text either way; everything else, in safe
  // preview, goes to the worker — which reads Word and web pages too.
  const safe = safePreview() && attachment.preview !== "text";
  status.textContent =
    attachment.preview === "none" && !safe
      ? t("kuverta can't show this kind of file itself. Save it, or open it in the app your computer has for it.")
      : t("Loading…");
  attachmentSheet.hidden = false;
  el("attachment-close").focus();
  if (safe) {
    try {
      await showSafePreview(generation, 0);
    } catch (err) {
      if (generation === viewer.generation) status.textContent = t("could not load it: {error}", { error: err });
    }
    return;
  }
  if (attachment.preview === "none") return;

  try {
    const bytes = new Uint8Array(await viewerCall("attachment"));
    if (generation !== viewer.generation || attachmentSheet.hidden) return;
    if (attachment.preview === "text") {
      const text = el("attachment-text");
      text.textContent = new TextDecoder().decode(bytes.subarray(0, 500_000));
      text.hidden = false;
    } else {
      viewer.url = URL.createObjectURL(new Blob([bytes], { type: attachment.content_type }));
      const target = el(attachment.preview === "pdf" ? "attachment-pdf" : "attachment-image");
      target.src = viewer.url;
      target.hidden = false;
    }
    status.textContent = "";
  } catch (err) {
    if (generation === viewer.generation) status.textContent = t("could not load it: {error}", { error: err });
  }
}

attachmentSheet.addEventListener("close", () => {
  viewer.generation += 1;
  clearViewer();
});
el("attachment-close").onclick = () => closeDialog(attachmentSheet);

el("attachment-save").onclick = async () => {
  try {
    const path = await viewerCall("save_attachment");
    say(t("saved to {path}", { path }));
  } catch (err) {
    say(t("could not save it: {error}", { error: err }), true);
  }
};

el("attachment-open").onclick = async () => {
  // In safe preview, another app is outside it: the first click says so, the
  // second opens.
  if (safePreview() && !viewer.openArmed) {
    viewer.openArmed = true;
    const open = el("attachment-open");
    open.textContent = t("Open anyway?");
    open.classList.add("danger");
    el("attachment-status").textContent = t(
      "Another app reads the file itself, outside safe preview. Open it only if you trust the sender — click again to open.",
    );
    return;
  }
  try {
    await viewerCall("open_attachment");
  } catch (err) {
    say(t("could not open it: {error}", { error: err }), true);
  }
};

// Settings → General → Attachments.
{
  const pref = el("pref-safe-preview");
  SETTINGS_PAGES.general.addEventListener("show", () => (pref.checked = safePreview()));
  pref.addEventListener("change", () => {
    try {
      localStorage.setItem("safePreview", pref.checked ? "yes" : "no");
    } catch {
      // Lasts until the window closes: the default, on, comes back then.
    }
  });
}
