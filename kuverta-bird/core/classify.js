/**
 * Deterministic classifier.
 *
 * This is the baseline the model has to beat. It is cheap, offline, produces
 * the same answer twice, and — the part that matters most — it can say *why*,
 * which is what makes a wrong answer correctable instead of infuriating.
 *
 * Signals are scored rather than matched first-to-win. Real mail carries
 * conflicting evidence (an invoice sent through a mailing list; a personal
 * reply from a `noreply` address), and a scoring model degrades sensibly where
 * an ordered rule list would need every conflict enumerated in the right
 * order. The matched signals are kept so the verdict stays explainable.
 *
 * One thing overrides the score: what you have told it before. A sender you
 * have corrected is filed the way you corrected it.
 *
 * Ported from `kuverta`'s `core-rules/src/lib.rs`. No Thunderbird API is
 * touched here on purpose: this file runs under plain Node for the tests, and
 * that is what keeps the port honest.
 */

import { Category } from './category.js';
import { evaluate } from './signals.js';

/**
 * @typedef {object} Reason
 * @property {string} rule    Stable identifier for the rule, for grouping and debugging.
 * @property {string} detail  Human-readable, shown in the UI under "why".
 * @property {string} category
 * @property {number} weight
 */

/**
 * @typedef {object} Classification
 * @property {string} category
 * @property {number} confidence 0.0–1.0. Derived from how far the winner led
 *   the runner-up, so a message with balanced evidence reports low confidence
 *   rather than pretending.
 * @property {Reason[]} reasons
 */

function normalize(value) {
  return value.trim().toLowerCase();
}

/**
 * Categories the user has previously assigned, keyed by sender and by list.
 *
 * Built from the corrections log. This is the whole learning mechanism at this
 * stage, and it is worth more than any heuristic: it is the one signal that is
 * definitely right.
 */
export class Learned {
  #bySender = new Map();
  #byList = new Map();

  static empty() {
    return new Learned();
  }

  withSender(addr, category) {
    this.#bySender.set(normalize(addr), category);
    return this;
  }

  withList(listId, category) {
    this.#byList.set(normalize(listId), category);
    return this;
  }

  insertSender(addr, category) {
    this.#bySender.set(normalize(addr), category);
  }

  insertList(listId, category) {
    this.#byList.set(normalize(listId), category);
  }

  get isEmpty() {
    return this.#bySender.size === 0 && this.#byList.size === 0;
  }

  /**
   * The overrides, for display.
   *
   * Exposed so the options page can show what was learned without folding the
   * corrections log a second time — two folds of the same log is two things to
   * keep in step, and only one of them would be tested.
   *
   * @returns {{senders: Array<[string, string]>, lists: Array<[string, string]>}}
   */
  entries() {
    return {
      senders: [...this.#bySender.entries()],
      lists: [...this.#byList.entries()],
    };
  }

  /** @returns {{category: string, detail: string, rule: string} | null} */
  lookup(facts) {
    // Sender first: it is more specific than the list it arrived through.
    if (facts.fromAddr != null) {
      const category = this.#bySender.get(normalize(facts.fromAddr));
      if (category !== undefined) {
        return {
          category,
          detail: `you have filed mail from ${facts.fromAddr} as ${category}`,
          rule: 'learned.sender',
        };
      }
    }
    if (facts.listId != null) {
      const category = this.#byList.get(normalize(facts.listId));
      if (category !== undefined) {
        return {
          category,
          detail: `you have filed mail from the list ${facts.listId} as ${category}`,
          rule: 'learned.list',
        };
      }
    }
    return null;
  }
}

/**
 * Score below which the classifier declines to guess.
 *
 * A confident wrong answer costs more than an honest "unknown": the first
 * hides mail in the wrong bucket, the second leaves it where you will see it.
 */
const MIN_SCORE = 1.0;

export class Classifier {
  #learned;

  constructor(learned = Learned.empty()) {
    this.#learned = learned;
  }

  static withoutHistory() {
    return new Classifier(Learned.empty());
  }

  /**
   * @param {import('./facts.js').MessageFacts} facts
   * @returns {Classification}
   */
  classify(facts) {
    // An explicit correction is not evidence to be weighed against
    // heuristics — it is the answer.
    const learned = this.#learned.lookup(facts);
    if (learned !== null) {
      return {
        category: learned.category,
        confidence: 1.0,
        reasons: [
          {
            rule: learned.rule,
            detail: learned.detail,
            category: learned.category,
            weight: Number.POSITIVE_INFINITY,
          },
        ],
      };
    }

    const reasons = evaluate(facts);

    const scores = new Map();
    for (const reason of reasons) {
      scores.set(reason.category, (scores.get(reason.category) ?? 0) + reason.weight);
    }

    // Sort by score, then by name so ties are stable across runs rather than
    // following map insertion order.
    const ranked = [...scores.entries()].sort(
      (a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0),
    );

    if (ranked.length === 0) {
      return { category: Category.Unknown, confidence: 0.0, reasons };
    }

    const [winner, top] = ranked[0];
    if (top < MIN_SCORE) {
      return { category: Category.Unknown, confidence: 0.0, reasons };
    }

    const runnerUp = ranked[1]?.[1] ?? 0.0;
    // 1.0 when nothing competed, 0.5 on a dead tie.
    const confidence = Math.min(1.0, Math.max(0.0, top / (top + runnerUp)));

    return {
      category: winner,
      confidence,
      // Keep only the evidence that supported the verdict; the rest is noise
      // in an explanation.
      reasons: reasons.filter((reason) => reason.category === winner),
    };
  }
}
