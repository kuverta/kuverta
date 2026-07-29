// Answer: the assistant writes what compose is open on.
//
// One click from the open message (✨ Answer, or a) opens the reply and has
// the assistant write it; in compose the same button writes whatever compose
// holds — a reply, a forward's note, a new message. What is typed in the body
// first is its brief: "zusagen, aber erst ab Mittwoch" is followed, a started
// reply is finished in its own words. Attached files are read too — the text
// of a PDF, a Word file, a scan through the vision model — so the answer can
// say what it is sending along.
//
// What it writes replaces the body through the field's own editing, so ⌘Z
// brings back what was typed. It changes no mail and sends nothing: see
// answer.rs, which offers the model only the tools that look.

const answerButton = el("compose-answer");
const answerStatus = el("compose-answer-status");

/// What the button says for what compose is open on.
function answerLabel() {
  return compose.replyTo !== null ? t("Answer with assistant") : t("Write with assistant");
}

function resetAnswerButton() {
  answerButton.disabled = false;
  answerButton.textContent = `✨ ${answerLabel()}`;
}

/// Puts `text` in the body as if typed, so the field's undo takes it back.
function replaceBody(text) {
  const body = compose.body;
  body.focus();
  body.select();
  // execCommand is the one way to change a textarea that its undo sees; a
  // window without it gets the text all the same, without the undo.
  let typed = false;
  try {
    typed = document.execCommand("insertText", false, text);
  } catch {
    typed = false;
  }
  if (!typed || body.value !== text) body.value = text;
  body.setSelectionRange(0, 0);
  body.scrollTop = 0;
}

/// Has the assistant write compose's body, taking what is there as its brief.
async function answerWithAssistant() {
  if (compose.pane.hidden || compose.answering) return;
  if (state.account === null) return;
  const ticket = {};
  compose.answering = ticket;
  answerButton.disabled = true;
  answerButton.textContent = t("Writing…");
  answerStatus.textContent = compose.files.files.length
    ? t("reading the attached files…")
    : t("the assistant is writing…");

  const channel = progressChannel((event) => {
    if (compose.answering === ticket && event.kind === "looked") answerStatus.textContent = `${event.what}…`;
  });
  try {
    const result = await invoke("draft_answer", {
      account: state.account,
      draft: draftInput(),
      onEvent: channel,
    });
    // Discarded, or opened on something else, while it wrote.
    if (compose.answering !== ticket) return;
    const hadNotes = compose.body.value.trim() !== "";
    replaceBody(`${result.body.trim()}\n`);
    answerStatus.textContent = hadNotes
      ? t("Written by {model}. Read it before you send it — ⌘Z brings back your notes.", { model: result.model })
      : t("Written by {model}. Read it before you send it.", { model: result.model });
  } catch (err) {
    if (compose.answering !== ticket) return;
    answerStatus.textContent = "";
    say(String(err), true);
  } finally {
    if (compose.answering === ticket) {
      compose.answering = null;
      resetAnswerButton();
    }
  }
}

/// From the list or the open message: the reply, written, in one step.
async function answerSelected() {
  if (state.postbox) {
    say(t("post cannot be answered from here"), true);
    return;
  }
  await openCompose({ replyAll: false });
  if (!compose.pane.hidden && compose.replyTo !== null) await answerWithAssistant();
}

answerButton.onclick = answerWithAssistant;

// The label follows what compose is open on; openCompose and openComposeWith
// set that up before showing the pane.
new MutationObserver(() => {
  if (!compose.pane.hidden && !compose.answering) resetAnswerButton();
}).observe(compose.pane, { attributes: true, attributeFilter: ["hidden"] });
