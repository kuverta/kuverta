// Files going out with a message: dropped on what is being written, or
// picked with Attach.
//
// Two places write mail — compose, and the reply box under a conversation —
// and each has a tray: a strip of chips, one per file, each with a × to take
// it back. The bytes are read as soon as a file is dropped and kept here as
// base64, which is what the core takes them as: a draft carries its files,
// so a message scheduled for Monday still has them on Monday.
//
// Dropping works because the window leaves file drops to the page
// (`dragDropEnabled: false` in tauri.conf.json); with Tauri's own handling on,
// a drop arrives as paths on a Tauri event and never as a drop here. The page
// then has to refuse a drop everywhere else, or a file let go of anywhere in
// the window would replace the window with the file.

/// Most mail servers refuse more than this in one message; the core refuses
/// it too, with the same number (session.rs, MAX_ATTACHMENT_BYTES).
const MOST_ATTACHMENT_BYTES = 25 * 1024 * 1024;

/// How many bytes base64 `data` stands for.
function base64Size(data) {
  const padding = data.endsWith("==") ? 2 : data.endsWith("=") ? 1 : 0;
  return Math.floor((data.length * 3) / 4) - padding;
}

/// A file's bytes as base64, without a data URL's prefix.
function readAsBase64(file) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result).slice(String(reader.result).indexOf(",") + 1));
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });
}

/// A tray of files for one place mail is written, drawn into `strip`.
function fileTray(strip) {
  const tray = {
    files: [],

    total() {
      return tray.files.reduce((sum, file) => sum + file.size, 0);
    },

    /// Reads and adds files from a drop or the picker. One too large for
    /// what is left is refused by name; the others still come in.
    async add(list) {
      for (const file of [...list]) {
        if (tray.total() + file.size > MOST_ATTACHMENT_BYTES) {
          say(
            t("{name} is too large: a message can carry {count} MB of files, and most servers refuse more", {
              name: file.name,
              count: MOST_ATTACHMENT_BYTES / 1024 / 1024,
            }),
            true,
          );
          continue;
        }
        try {
          const data = await readAsBase64(file);
          tray.files.push({ name: file.name, content_type: file.type, data, size: file.size });
        } catch (err) {
          // A folder dropped from Finder lands here too: it has no bytes.
          say(t("could not read {name}: {error}", { name: file.name, error: err }), true);
        }
      }
      tray.render();
    },

    /// Fills the tray from a draft the core kept.
    set(attachments = []) {
      tray.files = attachments.map((a) => ({ ...a, size: base64Size(a.data) }));
      tray.render();
    },

    clear() {
      tray.files = [];
      tray.render();
    },

    /// The files as a draft carries them.
    forDraft() {
      return tray.files.map(({ name, content_type, data }) => ({ name, content_type, data }));
    },

    render() {
      strip.textContent = "";
      strip.hidden = tray.files.length === 0;
      tray.files.forEach((file, at) => {
        const chip = document.createElement("span");
        chip.className = "attachment-chip outgoing";
        chip.title = file.name;
        const icon = document.createElement("span");
        icon.className = "icon";
        icon.textContent = attachmentIcon({
          content_type: file.content_type,
          preview: file.content_type.startsWith("image/")
            ? "image"
            : file.content_type === "application/pdf"
              ? "pdf"
              : "none",
        });
        const name = document.createElement("span");
        name.className = "name";
        name.textContent = file.name;
        const size = document.createElement("span");
        size.className = "size";
        size.textContent = sizeText(file.size);
        const remove = document.createElement("button");
        remove.type = "button";
        remove.className = "remove";
        remove.textContent = "×";
        remove.title = t("Take {name} out", { name: file.name });
        remove.setAttribute("aria-label", remove.title);
        remove.onclick = () => {
          tray.files.splice(at, 1);
          tray.render();
        };
        chip.append(icon, name, size, remove);
        strip.append(chip);
      });
    },
  };
  return tray;
}

/// Whether a drag carries files, rather than text or a link.
function dragHasFiles(event) {
  return [...(event.dataTransfer?.types ?? [])].includes("Files");
}

/// Lets files be dropped anywhere on `zone`, into `tray`. `ready` says
/// whether the zone takes them now — a hidden pane does not.
function acceptDrops(zone, tray, ready = () => !zone.hidden) {
  const hint = zone.querySelector(".drop-hint");
  // dragenter and dragleave fire for every child crossed; counting them is
  // the one way to know when the drag has really left.
  let depth = 0;
  const show = (on) => {
    zone.classList.toggle("dropping", on);
    if (hint) hint.hidden = !on;
  };
  zone.addEventListener("dragenter", (event) => {
    if (!dragHasFiles(event) || !ready()) return;
    event.preventDefault();
    depth += 1;
    show(true);
  });
  zone.addEventListener("dragover", (event) => {
    if (!dragHasFiles(event) || !ready()) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
  });
  zone.addEventListener("dragleave", () => {
    depth = Math.max(0, depth - 1);
    if (depth === 0) show(false);
  });
  zone.addEventListener("drop", async (event) => {
    depth = 0;
    show(false);
    if (!dragHasFiles(event) || !ready()) return;
    event.preventDefault();
    event.stopPropagation();
    await tray.add(event.dataTransfer.files);
  });
}

// Anywhere that does not take files, a dropped file is refused rather than
// opened in place of the window.
for (const type of ["dragover", "drop"]) {
  document.addEventListener(type, (event) => {
    if (!dragHasFiles(event) || event.defaultPrevented) return;
    event.preventDefault();
    if (type === "dragover") event.dataTransfer.dropEffect = "none";
  });
}

/// Opens the system's file picker for `tray`.
function pickFiles(tray) {
  const picker = el("file-picker");
  picker.value = "";
  picker.onchange = () => tray.add(picker.files);
  picker.click();
}

compose.files = fileTray(el("compose-files"));
acceptDrops(compose.pane, compose.files);
el("compose-attach").onclick = () => pickFiles(compose.files);
