/**
 * Thunderbird message -> the facts the classifier reads.
 *
 * The equivalent of `kuverta`'s `core-proto/src/parse.rs`, minus the MIME
 * parsing: Thunderbird has already parsed the message, so this file only has
 * to pick headers out and reshape them.
 *
 * `buildFacts` takes plain data rather than a Thunderbird object so it can be
 * tested under Node. The code that actually talks to the API lives in
 * `messages.js`.
 */

/**
 * @typedef {object} MessageFacts
 * @property {?string} fromAddr
 * @property {?string} fromName
 * @property {?string} subject
 * @property {?string} listId RFC 2919 `List-Id`, reduced to the bracketed identifier.
 * @property {?string} listUnsubscribe RFC 2369 `List-Unsubscribe`.
 * @property {?string} precedence `Precedence: bulk | list | junk`.
 * @property {?string} autoSubmitted RFC 3834 `Auto-Submitted`.
 * @property {?string} inReplyTo Set when the message is a reply.
 * @property {boolean} hasAttachments
 * @property {number} recipientCount How many addresses were on `To` and `Cc`.
 * @property {?string} snippet Leading plain text, used for body keywords.
 */

/**
 * Longest snippet kept. Enough for useful body keywords without pulling whole
 * bodies through the classifier.
 */
const SNIPPET_LEN = 220;

/** The default: every field absent, nothing attached, nobody addressed. */
export const EMPTY_FACTS = Object.freeze({
  fromAddr: null,
  fromName: null,
  subject: null,
  listId: null,
  listUnsubscribe: null,
  precedence: null,
  autoSubmitted: null,
  inReplyTo: null,
  hasAttachments: false,
  recipientCount: 0,
  snippet: null,
});

/**
 * @param {object} input
 * @param {?string} [input.subject]  Already RFC 2047-decoded by Thunderbird.
 * @param {?string} [input.author]   `MessageHeader.author`, e.g. `Anna Weber <anna@example.de>`.
 * @param {string[]} [input.recipients]
 * @param {string[]} [input.ccList]
 * @param {object} [input.headers]   `MessagePart.headers`: lowercased names -> array of values.
 * @param {boolean} [input.hasAttachments]
 * @param {?string} [input.bodyText]
 * @returns {MessageFacts}
 */
export function buildFacts({
  subject = null,
  author = null,
  recipients = [],
  ccList = [],
  headers = {},
  hasAttachments = false,
  bodyText = null,
} = {}) {
  const { name, addr } = splitAuthor(author);
  const rawListId = firstHeader(headers, 'list-id');

  return {
    fromAddr: addr,
    fromName: name,
    subject: blankToNull(subject),
    listId: rawListId === null ? null : cleanListId(rawListId),
    listUnsubscribe: firstHeader(headers, 'list-unsubscribe'),
    precedence: firstHeader(headers, 'precedence'),
    autoSubmitted: firstHeader(headers, 'auto-submitted'),
    inReplyTo: firstHeader(headers, 'in-reply-to'),
    hasAttachments,
    // Recipients across To and Cc, for the "addressed only to you" signal.
    recipientCount: (recipients?.length ?? 0) + (ccList?.length ?? 0),
    snippet: bodyText == null ? null : snippet(bodyText),
  };
}

/**
 * First value of a header, trimmed, or null.
 *
 * Thunderbird gives `MessagePart.headers` as arrays because a header may
 * legally repeat; every header this classifier reads is one where the first
 * occurrence is the one that counts.
 */
export function firstHeader(headers, name) {
  if (headers == null) return null;

  // Looked up case-insensitively rather than assuming a spelling. Thunderbird
  // lowercases these keys in practice, but the API only promises "the header
  // name as key" — and a header that is there under `List-Id` while this asks
  // for `list-id` does not fail loudly, it just silently turns off a signal.
  let value = headers[name];
  if (value === undefined) {
    const wanted = name.toLowerCase();
    for (const key of Object.keys(headers)) {
      if (key.toLowerCase() === wanted) {
        value = headers[key];
        break;
      }
    }
  }

  if (value == null) return null;
  const first = Array.isArray(value) ? value[0] : value;
  return blankToNull(typeof first === 'string' ? first.trim() : null);
}

/**
 * `List-Id: Rust Weekly <news.rustweekly.example>` -> `news.rustweekly.example`.
 *
 * The bracketed identifier is the stable part; the display name is not.
 */
export function cleanListId(raw) {
  const value = raw.trim();
  const open = value.lastIndexOf('<');
  const close = value.lastIndexOf('>');
  if (open !== -1 && close !== -1 && close > open + 1) {
    return value.slice(open + 1, close).trim();
  }
  return value;
}

/**
 * `Anna Weber <anna@example.de>` -> `{name: 'Anna Weber', addr: 'anna@example.de'}`.
 *
 * Deliberately small. This is not a general RFC 5322 address parser and must
 * not grow into one — hand-rolled address parsing is how mail clients get
 * bugs. It handles the two shapes Thunderbird actually hands back for
 * `author`: an addr-spec on its own, and a display name in front of an angle
 * address. Anything stranger falls through as an address with no name, which
 * costs one heuristic rather than producing a wrong one.
 */
export function splitAuthor(author) {
  if (author == null) return { name: null, addr: null };

  const value = author.trim();
  if (value === '') return { name: null, addr: null };

  const open = value.lastIndexOf('<');
  const close = value.lastIndexOf('>');
  if (open !== -1 && close > open) {
    const addr = value.slice(open + 1, close).trim();
    const name = unquote(value.slice(0, open).trim());
    return { name: blankToNull(name), addr: blankToNull(addr) };
  }

  return { name: null, addr: blankToNull(value) };
}

/** Strips the surrounding quotes RFC 5322 puts round a display name. */
function unquote(value) {
  if (value.length >= 2 && value.startsWith('"') && value.endsWith('"')) {
    return value.slice(1, -1).replace(/\\(.)/g, '$1').trim();
  }
  return value;
}

/** Collapses whitespace and truncates on a character boundary. */
export function snippet(body) {
  const collapsed = body.split(/\s+/).filter(Boolean).join(' ');
  const chars = [...collapsed];
  if (chars.length <= SNIPPET_LEN) return blankToNull(collapsed);
  return `${chars.slice(0, SNIPPET_LEN).join('')}…`;
}

function blankToNull(value) {
  return value == null || value === '' ? null : value;
}
