// Settings → General → Signature: the picture of the person's handwritten
// signature that the assistant puts on a PDF when asked to, and the place
// written in front of the date beneath it.
//
// The picture is read here as base64, the way a file attached to a message
// is, and kept by the core, which decodes it once to make sure it draws. It
// is fetched back only for the preview on this page; the chat never sees it,
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
    status.textContent = settings.present
      ? t("{width} × {height} pixels. Ask the assistant to sign a PDF, and it uses this.", {
          width: settings.width,
          height: settings.height,
        })
      : t("No signature yet. The assistant can write PDFs, but not sign them.");
    el("signature-place").value = settings.place ?? "";
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

  el("signature-place").addEventListener("change", async (event) => {
    try {
      await invoke("set_signature_place", { place: event.target.value.trim() });
    } catch (err) {
      say(String(err), true);
    }
  });
}
