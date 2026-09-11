/**
 * The corrections log, readable.
 *
 * Read-only on purpose. The log is append-only (a correction you reverse is
 * two events, not an edit), so there is nothing here to save — only something
 * to look at and to take away.
 */

import { Corrections, learnedFrom } from '../src/corrections.js';
import { CATEGORY_LABELS } from '../src/category.js';

const corrections = new Corrections(messenger.storage.local);
const events = await corrections.all();

document.getElementById('count').textContent = String(events.length);

// -- what it has learned ----------------------------------------------------

const { senders, lists } = learnedFrom(events).entries();
const overrides = [
  ...senders.map(([key, category]) => [key, category, 'sender']),
  ...lists.map(([key, category]) => [key, category, 'list']),
].sort((a, b) => a[0].localeCompare(b[0]));

const learnedBody = document.querySelector('#learned tbody');
if (overrides.length === 0) {
  document.getElementById('learned-empty').hidden = false;
} else {
  learnedBody.append(
    ...overrides.map(([key, category, kind]) => row([key, kind, pill(category)])),
  );
}

// -- the log ----------------------------------------------------------------

document.querySelector('#log tbody').append(
  ...[...events].reverse().map((event) =>
    row([
      new Date(event.at).toLocaleString(),
      event.subject ?? '(no subject)',
      event.fromAddr ?? event.listId ?? '—',
      event.was ? pill(event.was) : '—',
      pill(event.now),
    ]),
  ),
);

// -- export -----------------------------------------------------------------

document.getElementById('export').addEventListener('click', () => {
  const blob = new Blob([JSON.stringify(events, null, 2)], {
    type: 'application/json',
  });
  const url = URL.createObjectURL(blob);
  const link = document.createElement('a');
  link.href = url;
  link.download = `fuckbird-corrections-${new Date().toISOString().slice(0, 10)}.json`;
  link.click();
  URL.revokeObjectURL(url);
});

// -- helpers ----------------------------------------------------------------

function row(cells) {
  const tr = document.createElement('tr');
  for (const cell of cells) {
    const td = document.createElement('td');
    // textContent, never innerHTML: these strings are mail subjects and
    // sender addresses, which is to say they are attacker-controlled.
    if (cell instanceof Node) td.append(cell);
    else td.textContent = cell;
    tr.append(td);
  }
  return tr;
}

function pill(category) {
  const span = document.createElement('span');
  span.className = 'pill';
  span.dataset.category = category;
  span.textContent = CATEGORY_LABELS[category] ?? category;
  return span;
}
