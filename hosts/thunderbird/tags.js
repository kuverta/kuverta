/**
 * Categories as Thunderbird tags.
 *
 * Stage 1 of the brief: no fork, no new UI surface, the classifier's verdict
 * shown through something Thunderbird already has. Tags are the right carrier
 * because they are visible in the message list, they survive a restart, they
 * are searchable, and — the part that matters for stage 3 — the user can
 * already change them by hand.
 *
 * The tag is a *view* of the verdict, not the record of it. The record is the
 * corrections log. Anything that reads tags back as truth will be wrong the
 * first time another client touches them.
 */

import { ALL_CATEGORIES, CATEGORY_LABELS } from '../../core/category.js';

/**
 * Every tag this add-on owns starts with this.
 *
 * It is the only thing separating our tags from the user's own, so nothing may
 * strip or rewrite a tag that does not carry it.
 */
export const TAG_PREFIX = 'fuckbird-';

/**
 * Distinguishable at a glance and in both themes, and deliberately not a
 * traffic-light scheme — none of these categories is "bad".
 */
const TAG_COLOURS = {
  personal: '#3B7DD8',
  newsletter: '#7A5FB0',
  marketing: '#C4552D',
  transactional: '#2E8B57',
  notification: '#6E7B85',
  unknown: '#9A9A9A',
};

export const tagKeyFor = (category) => `${TAG_PREFIX}${category}`;

/** The category a tag key names, or null if the tag is not ours. */
export function categoryForTagKey(key) {
  if (!key.startsWith(TAG_PREFIX)) return null;
  const category = key.slice(TAG_PREFIX.length);
  return ALL_CATEGORIES.includes(category) ? category : null;
}

/**
 * Creates the six tags if they are not already there.
 *
 * Idempotent, and it must stay that way: it runs on every startup, because an
 * add-on cannot assume its install hook was the last thing to touch the tag
 * list.
 */
export async function ensureTags(messenger) {
  const existing = await messenger.messages.tags.list();
  const keys = new Set(existing.map((tag) => tag.key));

  for (const category of ALL_CATEGORIES) {
    const key = tagKeyFor(category);
    if (keys.has(key)) continue;
    try {
      await messenger.messages.tags.create(
        key,
        `fuckbird: ${CATEGORY_LABELS[category].toLowerCase()}`,
        TAG_COLOURS[category],
      );
    } catch (error) {
      // Most likely the key exists under a different name, which is fine —
      // the user is allowed to rename our tags.
      console.warn(`fuckbird: could not create the tag ${key}`, error);
    }
  }
}

/**
 * Files a message under one category, leaving every other tag alone.
 *
 * `messages.update` replaces the whole tag list, so the user's own tags have
 * to be read and written back. Getting this wrong quietly deletes their
 * tagging, which is exactly the class of mistake §3.5 is about.
 *
 * @returns {Promise<boolean>} Whether anything changed.
 */
export async function applyCategory(messenger, messageId, category) {
  const header = await messenger.messages.get(messageId);
  const current = header.tags ?? [];
  const wanted = tagKeyFor(category);

  const theirs = current.filter((key) => categoryForTagKey(key) === null);
  const next = [...theirs, wanted];

  // Nothing to do is the common case on a re-run; writing anyway would touch
  // every message's modification time for no reason.
  if (sameTags(current, next)) return false;

  await messenger.messages.update(messageId, { tags: next });
  return true;
}

/** The category currently filed on a message, if one of ours is. */
export function currentCategory(header) {
  for (const key of header.tags ?? []) {
    const category = categoryForTagKey(key);
    if (category !== null) return category;
  }
  return null;
}

function sameTags(a, b) {
  if (a.length !== b.length) return false;
  const left = [...a].sort();
  const right = [...b].sort();
  return left.every((value, index) => value === right[index]);
}
