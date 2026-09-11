/**
 * The key map.
 *
 * Shared between hosts so the muscle memory is, which is the whole reason this
 * is a file rather than a switch statement in each view. Brief §3.1 lists the
 * vocabulary; this is it, and it is the only place it is written down.
 */

import { ACTIONS } from './host.js';

/** Intents a key can express. The view decides how to carry them out. */
export const INTENT = Object.freeze({
  Down: 'down',
  Up: 'up',
  Open: 'open',
  Close: 'close',
  Archive: 'archive',
  Trash: 'trash',
  ToggleRead: 'toggleRead',
  Undo: 'undo',
  Sync: 'sync',
  Search: 'search',
  Compose: 'compose',
  Reply: 'reply',
  ReplyAll: 'replyAll',
  Forward: 'forward',
  Settings: 'settings',
  Explain: 'explain',
});

/**
 * Key to intent.
 *
 * Case matters: `R` is reply-all's neighbour, not a second binding for reply.
 */
export const KEYS = Object.freeze({
  j: INTENT.Down,
  ArrowDown: INTENT.Down,
  k: INTENT.Up,
  ArrowUp: INTENT.Up,
  Enter: INTENT.Open,
  o: INTENT.Open,
  Escape: INTENT.Close,
  e: INTENT.Archive,
  '#': INTENT.Trash,
  Delete: INTENT.Trash,
  u: INTENT.ToggleRead,
  z: INTENT.Undo,
  r: INTENT.Sync,
  '/': INTENT.Search,
  c: INTENT.Compose,
  R: INTENT.Reply,
  A: INTENT.ReplyAll,
  f: INTENT.Forward,
  ',': INTENT.Settings,
  '?': INTENT.Explain,
});

/** Intents that map straight onto a triage action. */
export const INTENT_ACTIONS = Object.freeze({
  [INTENT.Archive]: ACTIONS.Archive,
  [INTENT.Trash]: ACTIONS.Trash,
  [INTENT.ToggleRead]: ACTIONS.ToggleRead,
  [INTENT.Open]: ACTIONS.Open,
});

/**
 * What a key means here, or null if it means nothing.
 *
 * Returns null for any key carrying a modifier: `ctrl+r` is the host's reload
 * and `cmd+c` is copy, and stealing either would be rude. The exception is
 * nothing — a triage shortcut worth a modifier does not exist yet.
 *
 * @param {KeyboardEvent} event
 * @returns {?string} an `INTENT`
 */
export function intentFor(event) {
  if (event.ctrlKey || event.metaKey || event.altKey) return null;
  return KEYS[event.key] ?? null;
}

/**
 * Whether a key event belongs to whatever the user is typing into.
 *
 * Without this, typing `e` into a subject line archives something. It is the
 * single most damaging bug this file can have, so it is a function with a name
 * rather than a condition inlined at the call site.
 *
 * @param {EventTarget} target
 */
export function isTyping(target) {
  if (!target || typeof target !== 'object') return false;
  const tag = (target.tagName ?? '').toUpperCase();
  if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return true;
  return Boolean(target.isContentEditable);
}

/** The list shown by `?`, in the order it should be read. */
export const CHEAT_SHEET = Object.freeze([
  ['j / k', 'move down and up'],
  ['enter', 'read'],
  ['e', 'archive'],
  ['#', 'trash'],
  ['u', 'unread'],
  ['z', 'undo'],
  ['1…6', 'file as a category'],
  ['/', 'search'],
  ['r', 'sync'],
  ['c', 'compose'],
  ['R / A / f', 'reply, reply all, forward'],
  [',', 'settings'],
  ['?', 'this list'],
]);
