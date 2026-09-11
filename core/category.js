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

/**
 * What each category means, for the person reading it.
 *
 * The doc comments above say the same thing to whoever is reading the source.
 * These say it in the window, which is where the question actually gets asked —
 * a sidebar listing six words nobody defined is a filter you have to guess at.
 */
export const CATEGORY_MEANINGS = Object.freeze({
  [Category.Personal]: 'A human wrote this to you and expects an answer.',
  [Category.Newsletter]: 'Bulk mail you subscribed to: lists, digests, newsletters.',
  [Category.Marketing]: 'Bulk mail that wants you to buy something.',
  [Category.Transactional]: 'Money and records — invoices, receipts, orders, statements.',
  [Category.Notification]: 'Machine-generated status: builds, alerts, delivery notices.',
  [Category.Unknown]:
    'No signal strong enough to guess. Left here deliberately rather than filed on a hunch.',
});

/** For the UI: what the category means, in one line. */
export const CATEGORY_LABELS = Object.freeze({
  [Category.Personal]: 'Personal',
  [Category.Newsletter]: 'Newsletter',
  [Category.Marketing]: 'Marketing',
  [Category.Transactional]: 'Transactional',
  [Category.Notification]: 'Notification',
  [Category.Unknown]: 'Unknown',
});
