/**
 * Corrections are training data.
 *
 * Every reclassification is recorded from the first day, even though nothing
 * but the rules consumes it yet. By the time a model arrives there is a
 * labelled corpus of exactly the mail it will be asked about.
 *
 * What is stored is the *event*, not the current state — "on this date you
 * moved mail from this sender out of `newsletter` and into `transactional`" —
 * because the current state is recoverable from the events and the events are
 * not recoverable from the state. Thunderbird's own tags hold the state; they
 * are not a substitute for this.
 */

import { Learned } from './classify.js';
import { parseCategory } from './category.js';

const KEY = 'corrections';

/**
 * @typedef {object} Correction
 * @property {string} at           ISO 8601, when you corrected it.
 * @property {?string} messageId   RFC 5322 `Message-ID`. Legally absent on some mail (§3.6).
 * @property {?string} fromAddr
 * @property {?string} listId
 * @property {?string} subject     Kept for reading the log back; never classified on.
 * @property {?string} was         What the classifier said.
 * @property {string} now          What you said.
 * @property {?string} wasRule     Which rule produced `was`, when there was one.
 */

export class Corrections {
  #area;

  /** @param {{get: Function, set: Function}} area A `storage.local`-shaped area. */
  constructor(area) {
    this.#area = area;
  }

  /** @returns {Promise<Correction[]>} Oldest first. */
  async all() {
    const stored = await this.#area.get(KEY);
    const list = stored?.[KEY];
    return Array.isArray(list) ? list : [];
  }

  /**
   * Appends one correction.
   *
   * Append-only on purpose: a correction you later reverse is two events, not
   * an edit. Reversals are the most interesting rows in the log.
   *
   * @param {Correction} correction
   */
  async record(correction) {
    const list = await this.all();
    list.push({ at: new Date().toISOString(), ...correction });
    await this.#area.set({ [KEY]: list });
    return list.length;
  }

  /** The learned overrides implied by the log. @returns {Promise<Learned>} */
  async learned() {
    return learnedFrom(await this.all());
  }
}

/**
 * Folds the event log down to the overrides the classifier consults.
 *
 * Last write wins, which is what "a sender you have corrected is filed the way
 * you corrected it" means when you have corrected it twice.
 *
 * @param {Correction[]} events Oldest first.
 * @returns {Learned}
 */
export function learnedFrom(events) {
  const learned = Learned.empty();
  for (const event of events) {
    const category = parseCategory(event?.now);
    if (category === null) continue;
    // Sender and list are recorded together but are separate overrides: the
    // same correction teaches both, and either may match on its own later.
    if (event.fromAddr) learned.insertSender(event.fromAddr, category);
    if (event.listId) learned.insertList(event.listId, category);
  }
  return learned;
}
