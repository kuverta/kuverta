// The log, for a bug report: switching detailed logging on, reading the end
// of the log, and exporting it to attach. Also sends the window's own
// unexpected errors to it, so a log has both halves of what went wrong.

const logView = el("log-view");

async function fillDiagnostics() {
  try {
    const status = await invoke("log_status");
    el("log-detailed").checked = status.detailed;
    el("log-where").textContent = `Kept in ${status.path} (${formatLogSize(status.size_bytes)}).`;
  } catch (err) {
    el("log-where").textContent = String(err);
  }
  await refreshLog();
}

function formatLogSize(bytes) {
  if (bytes < 1024) return `${bytes} bytes`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

async function refreshLog() {
  try {
    logView.textContent = (await invoke("log_tail", { lines: 400 })) || "(the log is empty)";
    logView.scrollTop = logView.scrollHeight;
  } catch (err) {
    logView.textContent = String(err);
  }
}

el("log-detailed").addEventListener("change", async (event) => {
  try {
    await invoke("set_detailed_logging", { on: event.target.checked });
    say(event.target.checked ? "detailed logging is on" : "detailed logging is off");
    await fillDiagnostics();
  } catch (err) {
    event.target.checked = !event.target.checked;
    say(String(err), true);
  }
});

el("log-refresh").onclick = refreshLog;

el("log-copy").onclick = async () => {
  try {
    await navigator.clipboard.writeText(logView.textContent);
    say("copied");
  } catch {
    // No clipboard access: select it, so ⌘C does the rest.
    const range = document.createRange();
    range.selectNodeContents(logView);
    const selection = window.getSelection();
    selection.removeAllRanges();
    selection.addRange(range);
    say("selected — press ⌘C (ctrl+C) to copy");
  }
};

el("log-export").onclick = async () => {
  try {
    const path = await invoke("export_log");
    say(`saved to ${path}`);
  } catch (err) {
    say(`could not export the log: ${err}`, true);
  }
};

SETTINGS_PAGES.diagnostics.addEventListener("show", fillDiagnostics);

// Which version this is, at the foot of the settings list and in General.
invoke("log_status")
  .then((status) => {
    if (!status.version) return;
    el("settings-version").textContent = `kuverta ${status.version}`;
    el("settings-version-line").textContent = `This is kuverta ${status.version}.`;
  })
  .catch(() => {});

window.addEventListener("error", (event) => {
  invoke("log_ui", {
    level: "error",
    message: `${event.message} at ${event.filename}:${event.lineno}`,
  }).catch(() => {});
});

window.addEventListener("unhandledrejection", (event) => {
  invoke("log_ui", { level: "error", message: `unhandled: ${event.reason}` }).catch(() => {});
});
