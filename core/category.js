/**
 * The triage taxonomy.
 *
 * Deliberately small. Every category has to earn its place by changing what
 * you would *do* with a message — a taxonomy with twenty buckets is one
 * nobody keeps tidy, and the point of this tool is to reduce decisions.
 *
 * Ported from `fuckmail`'s `core-rules/src/category.rs`.
 */

export const Category = Object.freeze({
  /** A human wrote this to you and expects an answer. */
  Personal: 'personal',
  /** Subscribed bulk mail: lists, digests, newsletters. */
  Newsletter: 'newsletter',
  /** Unsubscribed or promotional bulk mail. */
  Marketing: 'marketing',
  /** Invoices, receipts, orders, bank statements — money and records. */
  Transactional: 'transactional',
  /** Machine-generated status: CI, cron, alerts, delivery notices. */
  Notification: 'notification',
  /** No signal strong enough to guess. Better than a confident wrong answer. */
  Unknown: 'unknown',
});

export const ALL_CATEGORIES = Object.freeze([
  Category.Personal,
  Category.Newsletter,
  Category.Marketing,
  Category.Transactional,
  Category.Notification,
  Category.Unknown,
]);

/** Returns the category if `value` names one, otherwise null. */
export function parseCategory(value) {
  return ALL_CATEGORIES.includes(value) ? value : null;
}

/** For the UI: what the category means, in one line. */
export const CATEGORY_LABELS = Object.freeze({
  [Category.Personal]: 'Personal',
  [Category.Newsletter]: 'Newsletter',
  [Category.Marketing]: 'Marketing',
  [Category.Transactional]: 'Transactional',
  [Category.Notification]: 'Notification',
  [Category.Unknown]: 'Unknown',
});
