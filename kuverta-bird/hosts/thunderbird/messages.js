/**
 * The Thunderbird side of building facts.
 *
 * Everything that touches the MailExtension API lives here, so `facts.js` and
 * `classify.js` stay plain data and stay testable under Node. If a Thunderbird
 * version changes an API shape, this is the only file that should need to know.
 */

import { buildFacts } from '../../core/facts.js';

/**
 * Longest body read for the snippet. The classifier only looks at the first
 * couple of hundred characters, and pulling whole bodies for a folder backfill
 * is the difference between a pause and a lockup.
 */
const MAX_BODY = 4000;

/**
 * Facts for one message, or null if it could not be read.
 *
 * Returns null rather than throwing: a backfill runs over thousands of
 * messages, and one unreadable message must not stop the rest.
 *
 * @param {object} messenger The `messenger` global.
 * @param {number} id A `MessageHeader.id` — a session id, not a stable one.
 */
export async function factsForMessage(messenger, id) {
  let header;
  let full;
  try {
    header = await messenger.messages.get(id);
    full = await messenger.messages.getFull(id);
  } catch (error) {
    console.warn(`kuverta-bird: could not read message ${id}`, error);
    return null;
  }

  return buildFacts({
    // Thunderbird has already decoded the RFC 2047 words in these.
    subject: header.subject ?? null,
    author: header.author ?? null,
    recipients: header.recipients ?? [],
    ccList: header.ccList ?? [],
    headers: full.headers ?? {},
    hasAttachments: hasAttachments(full),
    bodyText: bodyText(full),
  });
}

/**
 * Whether the message carries an attachment.
 *
 * Read off the part tree rather than by calling `listAttachments`, which would
 * be a second round trip per message for a signal worth 1.0 — and a backfill
 * makes that trade thousands of times.
 */
export function hasAttachments(part) {
  for (const child of walk(part)) {
    const disposition = (child.headers?.['content-disposition']?.[0] ?? '').toLowerCase();

    // The disposition is the sender saying which it is, so believe it.
    if (disposition.includes('attachment')) return true;
    if (disposition.includes('inline')) continue;

    // No disposition: a name is the remaining hint. Inline images carry one
    // too, so the content type has to rule out the parts that are the message
    // rather than something attached to it — otherwise every marketing mail
    // with a tracking pixel fires the attachment signal.
    if (!child.name) continue;
    const type = (child.contentType ?? '').toLowerCase();
    if (type.startsWith('multipart/') || type.startsWith('text/')) continue;
    return true;
  }
  return false;
}

/**
 * The text a reader would see: the plain part if there is one, otherwise the
 * HTML flattened.
 *
 * A lot of marketing mail is HTML-only, and skipping it would silently disable
 * every body signal on exactly the mail the body signals exist for.
 */
export function bodyText(part) {
  let plain = null;
  let markup = null;

  for (const child of walk(part)) {
    if (typeof child.body !== 'string' || child.body === '') continue;
    const type = (child.contentType ?? '').toLowerCase();
    if (type.startsWith('text/plain') && plain === null) plain = child.body;
    else if (type.startsWith('text/html') && markup === null) markup = child.body;
  }

  if (plain !== null) return plain.slice(0, MAX_BODY);
  if (markup === null) return null;
  return stripMarkup(markup.slice(0, MAX_BODY * 4)).slice(0, MAX_BODY);
}

/**
 * Tags out, entities in.
 *
 * Not a renderer and must not become one — the classifier reads this for
 * keywords, and "close enough to match `rabatt`" is the whole requirement.
 */
function stripMarkup(html) {
  return html
    .replace(/<(script|style)\b[^>]*>[\s\S]*?<\/\1>/gi, ' ')
    .replace(/<[^>]*>/g, ' ')
    .replace(/&nbsp;/gi, ' ')
    .replace(/&amp;/gi, '&')
    .replace(/&lt;/gi, '<')
    .replace(/&gt;/gi, '>')
    .replace(/&quot;/gi, '"')
    .replace(/&#(\d+);/g, (_, code) => String.fromCodePoint(Number(code)));
}

/** Depth-first walk of a `MessagePart` tree, the root included. */
function* walk(part) {
  if (part == null) return;
  yield part;
  for (const child of part.parts ?? []) yield* walk(child);
}

/**
 * Every message in a folder, a page at a time.
 *
 * Thunderbird pages message lists, and a folder with tens of thousands of
 * messages will not arrive in one call.
 */
export async function* eachMessage(messenger, folder) {
  let page = await messenger.messages.list(folder);
  for (;;) {
    for (const header of page.messages) yield header;
    if (!page.id) return;
    page = await messenger.messages.continueList(page.id);
    if (!page || page.messages.length === 0) return;
  }
}
