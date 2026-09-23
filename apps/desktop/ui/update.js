// Telling people a newer kuverta has been released.
//
// Asked shortly after the window opens and twice a day after that. The core
// asks GitHub; this shows a line in the header with a download link. A release
// put aside with "Later" is not mentioned again, though the next one is.
//
// Loaded after app.js, whose `invoke`, `el` and `say` it uses.

const UPDATE_EVERY_MS = 12 * 60 * 60 * 1000;
const DISMISSED_KEY = "updateDismissed";

const updateBanner = {
  box: el("update-banner"),
  text: el("update-text"),
  info: null,
};

function dismissedUpdate() {
  try {
    return localStorage.getItem(DISMISSED_KEY);
  } catch {
    return null;
  }
}

function showUpdate(info) {
  updateBanner.info = info;
  updateBanner.text.textContent = t("kuverta {latest} is available (you have {current})", {
    latest: info.latest,
    current: info.current,
  });
  el("update-download").hidden = !info.download_url;
  updateBanner.box.hidden = false;
}

async function checkForUpdate({ manual = false } = {}) {
  let info;
  try {
    info = await invoke("check_for_update", { manual });
  } catch (err) {
    if (manual) say(String(err), true);
    return;
  }
  if (!info) return;
  if (info.newer && (manual || dismissedUpdate() !== info.latest)) {
    showUpdate(info);
    if (manual) say(t("kuverta {latest} is available", { latest: info.latest }));
  } else if (manual) {
    say(
      info.newer
        ? t("kuverta {latest} is available", { latest: info.latest })
        : t("kuverta {current} is the latest version", { current: info.current }),
    );
  }
}

const openUpdateLink = (url) => invoke("open_external", { url }).catch((err) => say(String(err), true));

el("update-download").onclick = () => {
  const info = updateBanner.info;
  if (info?.download_url) openUpdateLink(info.download_url);
};
el("update-notes").onclick = () => {
  if (updateBanner.info) openUpdateLink(updateBanner.info.url);
};
el("update-later").onclick = () => {
  try {
    localStorage.setItem(DISMISSED_KEY, updateBanner.info?.latest ?? "");
  } catch {
    // Without storage it asks again next time, which is harmless.
  }
  updateBanner.box.hidden = true;
};
el("settings-update").onclick = () => checkForUpdate({ manual: true });

// Not in the first seconds, which belong to opening the mailbox.
setTimeout(checkForUpdate, 5000);
setInterval(checkForUpdate, UPDATE_EVERY_MS);
