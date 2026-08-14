// Settings → General → Signature: the picture of the person's handwritten
// signature that the assistant puts on a PDF when asked to, and the place
// written in front of the date beneath it.
//
// The picture is read here as base64, the way a file attached to a message
// is, and kept by the core, which cuts the ink out of its paper (core-rpc's
// signature.rs). What comes back is the cut-out, shown on paper with the
// place and date line beneath it, as it is placed; Show on a page opens a
// sample PDF signed with it in the viewer. The chat never sees the picture,
// and the signed PDFs are the core's to make.

{
  const page = SETTINGS_PAGES.general;
  const status = el("signature-status");

  function renderSignature(settings) {
    const preview = el("signature-preview");
    const image = el("signature-image");
    preview.hidden = !settings.present;
    if (settings.present) {
      image.src = `data:${settings.content_type};base64,${settings.image}`;
    } else {
      image.removeAttribute("src");
    }
    el("signature-remove").hidden = !settings.present;
    el("signature-sample").hidden = !settings.present;
    status.textContent = settings.present
      ? t("Cut out of its paper, {width} × {height} pixels. Ask the assistant to sign a PDF, and this goes on it.", {
          width: settings.width,
          height: settings.height,
        })
      : t("No signature yet. The assistant can write PDFs, but not sign them.");
    el("signature-place").value = settings.place ?? "";
    showCaption();
  }

  /// The line beneath the signature as it will be written: the place, a
  /// comma, today's date — or the date alone.
  function showCaption() {
    const place = el("signature-place").value.trim();
    const now = new Date();
    const date = `${String(now.getDate()).padStart(2, "0")}.${String(now.getMonth() + 1).padStart(2, "0")}.${now.getFullYear()}`;
    el("signature-caption").textContent = place ? `${place}, ${date}` : date;
  }

  async function showSignature() {
    try {
      renderSignature(await invoke("signature"));
    } catch (err) {
      status.textContent = String(err);
    }
  }

  page.addEventListener("show", showSignature);

  el("signature-choose").onclick = () => el("signature-file").click();

  el("signature-file").addEventListener("change", async (event) => {
    const picker = event.target;
    const file = picker.files?.[0];
    picker.value = "";
    if (!file) return;
    try {
      const image = await readAsBase64(file);
      renderSignature(await invoke("set_signature", { image, contentType: file.type }));
      say(t("signature kept"));
    } catch (err) {
      say(String(err), true);
    }
  });

  el("signature-remove").onclick = async () => {
    if (!(await ask(t("Remove the signature? The assistant can then no longer sign anything."), { yes: t("Remove"), danger: true }))) return;
    try {
      await invoke("clear_signature");
      await showSignature();
    } catch (err) {
      say(String(err), true);
    }
  };

  el("signature-place").addEventListener("input", showCaption);
  el("signature-place").addEventListener("change", async (event) => {
    try {
      await invoke("set_signature_place", { place: event.target.value.trim() });
    } catch (err) {
      say(String(err), true);
    }
  });

  el("signature-sample").onclick = () => openSignatureSample();
}
