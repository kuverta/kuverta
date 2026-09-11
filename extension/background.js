/**
 * Wiring.
 *
 * Nothing here decides anything: it reads messages, hands the facts to the
 * classifier, and writes the verdict out as a tag. Every judgement lives in
 * `src/`, where it can be tested without Thunderbird.
 *
 * Stage 1 of the brief — the classifier as a MailExtension, categories shown
 * as tags, no fork — plus the corrections log of §3.4, which is recorded from
 * the first day because a corpus you did not start collecting is one you
 * cannot go back for.
 */

import { Classifier } from './src/classify.js';
import { Corrections } from './src/corrections.js';
import { factsForMessage, eachMessage } from './src/messages.js';
import { ensureTags, applyCategory, currentCategory } from './src/tags.js';
import { ALL_CATEGORIES, CATEGORY_LABELS, parseCategory } from './src/category.js';

const corrections = new Corrections(messenger.storage.local);

/**
 * Rebuilt whenever the corrections log changes rather than consulted through
 * storage on every message: classification runs per message during a sync, and
 * an async read in that path is a stall nobody asked for.
 */
let classifier = new Classifier();

async function reloadLearned() {
  classifier = new Classifier(await corrections.learned());
}

/**
 * Classifies one message and files it.
 *
 * @returns {Promise<?{category: string, confidence: number, reasons: object[]}>}
 */
async function classifyAndFile(messageId) {
  const facts = await factsForMessage(messenger, messageId);
  if (facts === null) return null;

  const verdict = classifier.classify(facts);
  try {
    await applyCategory(messenger, messageId, verdict.category);
  } catch (error) {
    console.warn(`fuckbird: could not tag message ${messageId}`, error);
  }
  return verdict;
}

// -- new mail ---------------------------------------------------------------

messenger.messages.onNewMailReceived.addListener(async (_folder, messageList) => {
  for (const header of messageList?.messages ?? []) {
    await classifyAndFile(header.id);
  }
});

// -- backfilling a folder ---------------------------------------------------

/** Guards against a second backfill starting on top of the first. */
let backfilling = false;

async function backfillFolder(folder) {
  if (backfilling) {
    await notify('Still working', 'A folder is already being classified.');
    return;
  }
  backfilling = true;

  let seen = 0;
  let filed = 0;
  try {
    for await (const header of eachMessage(messenger, folder)) {
      seen += 1;
      const verdict = await classifyAndFile(header.id);
      if (verdict !== null) filed += 1;
      // Yield to the UI every so often. A folder backfill is the one thing
      // here that can run for minutes, and Thunderbird staying responsive
      // through it is not optional.
      if (seen % 50 === 0) await new Promise((resolve) => setTimeout(resolve, 0));
    }
    await notify('Classified', `${filed} of ${seen} messages in ${folder.name}.`);
  } catch (error) {
    console.error('fuckbird: backfill failed', error);
    await notify('Stopped', `After ${seen} messages: ${error.message}`);
  } finally {
    backfilling = false;
  }
}

// -- corrections ------------------------------------------------------------

/**
 * Records that the user disagreed, then acts on it.
 *
 * The order matters: the log is written before the tag, so a crash between the
 * two loses a tag you can recompute rather than a correction you cannot.
 */
async function correct(messageId, category) {
  const header = await messenger.messages.get(messageId);
  const facts = await factsForMessage(messenger, messageId);
  // What the classifier would say right now, which is what the correction is
  // a correction *of*. Computed before the log is written, because writing it
  // changes the answer.
  const verdict = facts === null ? null : classifier.classify(facts);

  await corrections.record({
    messageId: header.headerMessageId ?? null,
    fromAddr: facts?.fromAddr ?? null,
    listId: facts?.listId ?? null,
    subject: header.subject ?? null,
    // The filed tag if there is one, else what the rules say — an untagged
    // message still disagreed with something.
    was: currentCategory(header) ?? verdict?.category ?? null,
    now: category,
    wasRule: verdict?.reasons[0]?.rule ?? null,
  });

  await reloadLearned();
  await applyCategory(messenger, messageId, category);
}

// -- menus ------------------------------------------------------------------

async function buildMenus() {
  // Cleared first because this runs on every background page load, and
  // `menus.create` with an id that already exists is an error rather than a
  // no-op. Rebuilding is cheap; guessing whether they survived is not.
  await messenger.menus.removeAll();

  messenger.menus.create({
    id: 'fuckbird-file-as',
    title: 'fuckbird: file as',
    contexts: ['message_list'],
  });

  for (const category of ALL_CATEGORIES) {
    messenger.menus.create({
      id: `fuckbird-file-as-${category}`,
      parentId: 'fuckbird-file-as',
      title: CATEGORY_LABELS[category],
      contexts: ['message_list'],
    });
  }

  messenger.menus.create({
    id: 'fuckbird-classify-folder',
    title: 'fuckbird: classify this folder',
    contexts: ['folder_pane'],
  });
}

messenger.menus.onClicked.addListener(async (info) => {
  if (info.menuItemId === 'fuckbird-classify-folder') {
    const folder = info.selectedFolder ?? info.displayedFolder;
    if (folder) await backfillFolder(folder);
    return;
  }

  const category = parseCategory(
    String(info.menuItemId).replace('fuckbird-file-as-', ''),
  );
  if (category === null) return;

  for (const header of info.selectedMessages?.messages ?? []) {
    await correct(header.id, category);
  }
});

// -- the "why" popup --------------------------------------------------------

messenger.runtime.onMessage.addListener((request) => {
  if (request?.type === 'explain') return explainDisplayed();
  if (request?.type === 'correct') {
    return correct(request.messageId, request.category).then(() => explainDisplayed());
  }
  return undefined;
});

/**
 * The verdict for whatever message is on screen.
 *
 * Recomputed rather than remembered. Message ids are session-scoped, so a
 * cache keyed on them would be wrong after a restart, and recomputing is
 * cheap — this is a `HashMap` lookup and some string matching.
 */
async function explainDisplayed() {
  const [tab] = await messenger.tabs.query({ active: true, currentWindow: true });
  if (!tab) return null;

  const header = await messenger.messageDisplay.getDisplayedMessage(tab.id);
  if (!header) return null;

  const facts = await factsForMessage(messenger, header.id);
  if (facts === null) return null;

  return {
    messageId: header.id,
    filed: currentCategory(header),
    ...classifier.classify(facts),
  };
}

// -- startup ----------------------------------------------------------------

async function start() {
  await ensureTags(messenger);
  await reloadLearned();
  await buildMenus();
}

// Once, here. The background page is loaded on install and on every startup,
// so there is no second hook to add — adding one would just run this twice.
start().catch((error) => console.error('fuckbird: failed to start', error));

async function notify(title, message) {
  try {
    await messenger.notifications.create({
      type: 'basic',
      title: `fuckbird — ${title}`,
      message,
    });
  } catch {
    // Notifications are a courtesy; never let one fail an operation.
    console.info(`fuckbird: ${title} — ${message}`);
  }
}
