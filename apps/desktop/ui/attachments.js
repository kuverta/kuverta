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

const attachmentSheet = el("attachment-sheet");
const viewer = { account: null, messageId: null, attachment: null, url: null, generation: 0 };

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
  const status = el("attachment-status");
  status.textContent =
    attachment.preview === "none"
      ? t("kuverta can't show this kind of file itself. Save it, or open it in the app your computer has for it.")
      : t("Loading…");
  attachmentSheet.hidden = false;
  el("attachment-close").focus();
  if (attachment.preview === "none") return;

  try {
    const bytes = new Uint8Array(
      await invoke("attachment", { account, id: messageId, index: attachment.index }),
    );
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
  const { account, messageId, attachment } = viewer;
  try {
    const path = await invoke("save_attachment", { account, id: messageId, index: attachment.index });
    say(t("saved to {path}", { path }));
  } catch (err) {
    say(t("could not save it: {error}", { error: err }), true);
  }
};

el("attachment-open").onclick = async () => {
  const { account, messageId, attachment } = viewer;
  try {
    await invoke("open_attachment", { account, id: messageId, index: attachment.index });
  } catch (err) {
    say(t("could not open it: {error}", { error: err }), true);
  }
};
