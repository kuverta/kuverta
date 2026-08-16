// Where the signature goes: the page, and the signature dragged onto its line.
//
// kuverta reads a contract for the line it rules for a signature and puts the
// signature there. When it cannot — a form that rules no line, a page laid out
// in a way nothing could read — the person says where instead of describing it
// in words to a model that cannot see the page.
//
// So: the page as a picture, drawn by safe preview out of the *original*, with
// the signature on top of it to drag and a slider for how wide. Saving signs
// the original again at that place, into the same document — the same id, the
// same name, so a chip already in a message being written still points at it.
// Nothing is sent; what changes is the file on the card.

const placeSheet = el("place-sheet");
const placePage = el("place-page");
const placeImage = el("place-sheet-image");
const placeSignature = el("place-signature");
const placeWidth = el("place-width");
const placeStatus = el("place-status");
const placeWhich = el("place-which");

/// What is open in the placer: the document, where its signature stands, and
/// how large each page is, so a place on the picture is millimetres.
const placing = {
  doc: null,
  at: null,
  sizes: [],
  saved: null,
};

/// Whether this document's signature can be moved at all.
function canPlace(doc) {
  return Boolean(doc?.placed);
}

/// The page of the original as a picture, drawn by safe preview. The signed
/// copy would show the signature twice: the one already on it, and the one
/// being dragged.
async function placePicture(page) {
  const source = placing.doc.placed.source;
  const preview =
    source.kind === "attachment"
      ? await invoke("attachment_preview", {
          account: state.account,
          id: source.message,
          index: source.index,
          first: page - 1,
          count: 1,
        })
      : await invoke("document_preview", { id: source.id, first: page - 1, count: 1 });
  if (preview.kind !== "pages" || !preview.pages.length) {
    throw new Error(preview.why ?? t("this page could not be drawn"));
  }
  return { picture: `data:image/png;base64,${preview.pages[0]}`, total: preview.total };
}

/// Draws the signature where `placing.at` says, as a fraction of the page.
function drawPlacement() {
  const [pageWidth, pageHeight] = placing.sizes[placing.at.page - 1] ?? [210, 297];
  const ratio = placing.signatureRatio ?? 0.3;
  const widthMm = placing.at.width_mm;
  const heightMm = widthMm * ratio;
  placeSignature.style.width = `${(widthMm / pageWidth) * 100}%`;
  placeSignature.style.left = `${(placing.at.x_mm / pageWidth) * 100}%`;
  // above_bottom_mm is to the signature's lower edge; CSS wants the upper one.
  const topMm = pageHeight - placing.at.above_bottom_mm - heightMm;
  placeSignature.style.top = `${(topMm / pageHeight) * 100}%`;
  placeWhich.textContent = t("Page {page} of {total}", {
    page: placing.at.page,
    total: Math.max(placing.sizes.length, 1),
  });
  el("place-previous").disabled = placing.at.page <= 1;
  el("place-next").disabled = placing.at.page >= placing.sizes.length;
}

/// Shows a page, with the signature on it where it now stands.
async function showPlacePage(page) {
  placeStatus.textContent = t("drawing the page…");
  try {
    const { picture } = await placePicture(page);
    placeImage.src = picture;
    placing.at = { ...placing.at, page };
    drawPlacement();
    placeStatus.textContent = "";
  } catch (err) {
    placeStatus.textContent = String(err);
  }
}

/// Opens the placer on a signed document. `onSaved` hears the document back
/// with the signature where the person put it.
async function openPlacer(doc, onSaved) {
  if (!canPlace(doc)) {
    say(t("the signature on {name} cannot be moved: kuverta no longer has what it was signed from", { name: doc.name }), true);
    return;
  }
  placing.doc = doc;
  placing.at = { ...doc.placed };
  placing.saved = onSaved ?? null;
  el("place-title").textContent = doc.name;
  placeSheet.hidden = false;
  placeStatus.textContent = "";

  const signature = await invoke("signature");
  if (!signature.present || !signature.image) {
    placeStatus.textContent = t("there is no signature stored — add one under Settings → General");
    return;
  }
  placeSignature.src = `data:${signature.content_type ?? "image/png"};base64,${signature.image}`;
  placing.signatureRatio = signature.width ? signature.height / signature.width : 0.3;
  placing.sizes = await invoke("document_pages", { id: doc.id });
  placeWidth.value = String(Math.round(placing.at.width_mm));
  await showPlacePage(placing.at.page);
}

function closePlacer() {
  placeSheet.hidden = true;
  placing.doc = null;
  placing.saved = null;
  placeImage.removeAttribute("src");
}

// Dragging: the signature follows the pointer and stays on the page.
let dragging = null;
placeSignature.addEventListener("pointerdown", (event) => {
  if (!placing.doc) return;
  event.preventDefault();
  const box = placeSignature.getBoundingClientRect();
  dragging = { dx: event.clientX - box.left, dy: event.clientY - box.top };
  placeSignature.setPointerCapture(event.pointerId);
});
placeSignature.addEventListener("pointermove", (event) => {
  if (!dragging || !placing.doc) return;
  const page = placeImage.getBoundingClientRect();
  const [pageWidth, pageHeight] = placing.sizes[placing.at.page - 1] ?? [210, 297];
  const perMmX = page.width / pageWidth;
  const perMmY = page.height / pageHeight;
  const left = event.clientX - dragging.dx - page.left;
  const top = event.clientY - dragging.dy - page.top;
  // Measured from what will be saved rather than from the drawn box: the
  // two differ by a rounded pixel, and the rounding is what would push a
  // signature dragged to the edge off the paper.
  const widthMm = placing.at.width_mm;
  const heightMm = widthMm * (placing.signatureRatio ?? 0.3);
  placing.at.x_mm = Math.min(Math.max(left / perMmX, 0), pageWidth - widthMm);
  placing.at.above_bottom_mm = Math.min(
    Math.max(pageHeight - top / perMmY - heightMm, 0),
    pageHeight - heightMm,
  );
  drawPlacement();
});
for (const done of ["pointerup", "pointercancel"]) {
  placeSignature.addEventListener(done, () => {
    dragging = null;
  });
}

placeWidth.oninput = () => {
  if (!placing.doc) return;
  placing.at.width_mm = Number(placeWidth.value);
  drawPlacement();
};

el("place-previous").onclick = () => showPlacePage(Math.max(1, placing.at.page - 1));
el("place-next").onclick = () => showPlacePage(Math.min(placing.sizes.length, placing.at.page + 1));
el("place-cancel").onclick = closePlacer;
placeSheet.onclick = (event) => {
  if (event.target === placeSheet) closePlacer();
};

el("place-save").onclick = async (event) => {
  const button = event.currentTarget;
  button.disabled = true;
  placeStatus.textContent = t("signing it again where you put it…");
  try {
    const document = await invoke("place_signature", {
      account: state.account,
      id: placing.doc.id,
      at: {
        ...placing.at,
        x_mm: Math.round(placing.at.x_mm * 10) / 10,
        above_bottom_mm: Math.round(placing.at.above_bottom_mm * 10) / 10,
        width_mm: Math.round(placing.at.width_mm * 10) / 10,
      },
    });
    placing.saved?.(document);
    closePlacer();
    say(t("the signature is on page {page} now", { page: document.placed?.page ?? 1 }));
  } catch (err) {
    placeStatus.textContent = String(err);
  } finally {
    button.disabled = false;
  }
};
