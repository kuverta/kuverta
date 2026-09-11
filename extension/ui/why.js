/**
 * The "why" popup.
 *
 * Asks the background page rather than classifying here, so there is one
 * classifier holding one view of the corrections log. A second copy in the
 * popup would drift the moment a correction landed.
 */

import { ALL_CATEGORIES, CATEGORY_LABELS } from '../src/category.js';

const categoryEl = document.getElementById('category');
const confidenceEl = document.getElementById('confidence');
const reasonsEl = document.getElementById('reasons');
const buttonsEl = document.getElementById('buttons');

let current = null;

for (const category of ALL_CATEGORIES) {
  const button = document.createElement('button');
  button.textContent = CATEGORY_LABELS[category];
  button.dataset.category = category;
  button.addEventListener('click', async () => {
    if (current === null) return;
    setBusy(true);
    render(
      await messenger.runtime.sendMessage({
        type: 'correct',
        messageId: current.messageId,
        category,
      }),
    );
    setBusy(false);
  });
  buttonsEl.append(button);
}

render(await messenger.runtime.sendMessage({ type: 'explain' }));

function render(verdict) {
  current = verdict;

  if (!verdict) {
    categoryEl.textContent = 'no message';
    confidenceEl.textContent = '';
    reasonsEl.replaceChildren();
    return;
  }

  categoryEl.textContent = CATEGORY_LABELS[verdict.category];
  categoryEl.dataset.category = verdict.category;

  confidenceEl.textContent =
    verdict.category === 'unknown'
      ? 'nothing to go on'
      : `${Math.round(verdict.confidence * 100)}% confident`;

  reasonsEl.replaceChildren(
    ...verdict.reasons.map((reason) => {
      const item = document.createElement('li');

      const detail = document.createElement('span');
      detail.textContent = reason.detail;
      item.append(detail);

      const weight = document.createElement('span');
      weight.className = 'weight';
      // A learned override has no meaningful weight — it is not evidence
      // being weighed, it is the answer.
      weight.textContent = Number.isFinite(reason.weight)
        ? `+${reason.weight}`
        : 'your call';
      item.append(weight);

      return item;
    }),
  );

  // The category it is already filed under is not a correction.
  for (const button of buttonsEl.children) {
    button.disabled = button.dataset.category === verdict.category;
  }
}

function setBusy(busy) {
  document.body.classList.toggle('busy', busy);
}
