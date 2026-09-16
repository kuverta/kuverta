/**
 * Enough of Thunderbird to run the host contract against.
 *
 * Faithful about the two things that actually bite:
 *
 * **A move issues a new id.** The moved message comes back with an id it did
 * not have before, and the old one stops resolving. An adapter that assumes
 * otherwise passes a friendlier fake and fails against Thunderbird.
 *
 * **`query` spans folders and is not sorted.** Rows come back in insertion
 * order, so anything that needs them newest-first has to sort them itself.
 * The fake deliberately seeds them out of order for that reason.
 *
 * It is not a mail client. It models the calls this adapter makes, and a call
 * it does not model is one nothing is claiming to have tested.
 */

const INBOX = { id: 'f-inbox', name: 'Inbox', path: '/INBOX', specialUse: ['inbox'] };
const ARCHIVE = { id: 'f-archive', name: 'Archive', path: '/Archive', specialUse: ['archives'] };
const TRASH = { id: 'f-trash', name: 'Trash', path: '/Trash', specialUse: ['trash'] };

export function fakeMessenger({ seed = defaultSeed(), folders = [INBOX, ARCHIVE, TRASH] } = {}) {
  const messages = new Map();
  let nextId = 1;

  const put = (message) => {
    const id = nextId++;
    messages.set(id, { ...message, id });
    return id;
  };

  for (const message of seed) put(message);

  const need = (id) => {
    const message = messages.get(Number(id));
    if (!message) throw new Error(`no such message: ${id}`);
    return message;
  };

  const asHeader = (message) => ({
    id: message.id,
    subject: message.subject,
    author: message.author,
    date: new Date(message.date),
    read: message.read,
    tags: [...message.tags],
    headerMessageId: message.headerMessageId,
    folder: folders.find((f) => f.id === message.folderId),
    attachments: message.attachments ?? [],
  });

  return {
    // Exposed for assertions; not part of the API being faked.
    _messages: messages,
    _folders: folders,

    accounts: {
      list: async () => [{ id: 'a1', name: 'Test', folders }],
    },

    messages: {
      query: async (q) => {
        let found = [...messages.values()];
        if (q.folderId) found = found.filter((m) => m.folderId === q.folderId);
        if (q.headerMessageId) {
          found = found.filter((m) => m.headerMessageId === q.headerMessageId);
        }
        if (q.tags?.tags) {
          const wanted = Object.keys(q.tags.tags);
          found = found.filter((m) => wanted.some((tag) => m.tags.includes(tag)));
        }
        if (q.fullText) {
          const needle = String(q.fullText).toLowerCase();
          found = found.filter(
            (m) =>
              (m.subject ?? '').toLowerCase().includes(needle) ||
              (m.body ?? '').toLowerCase().includes(needle),
          );
        }
        // No ordering promise, and no pagination in the fake: one page, and
        // `id: null` to say there is no more.
        return { id: null, messages: found.map(asHeader) };
      },

      continueList: async () => ({ id: null, messages: [] }),

      get: async (id) => asHeader(need(id)),

      getFull: async (id) => {
        const message = need(id);
        return {
          contentType: 'text/plain',
          headers: { subject: [message.subject ?? ''] },
          body: message.body ?? '',
          parts: [],
        };
      },

      update: async (id, changes) => {
        const message = need(id);
        if ('read' in changes) message.read = changes.read;
        if ('tags' in changes) message.tags = [...changes.tags];
      },

      move: async (ids, folder) => {
        const target = folder.id ?? folder;
        for (const id of ids) {
          const message = need(id);
          messages.delete(message.id);
          // The new id is what makes this fake worth having.
          put({ ...message, folderId: target, id: undefined });
        }
      },

      tags: {
        list: async () => [],
        create: async () => {},
      },
    },
  };
}

/**
 * Six messages with distinct subjects, seeded out of date order so that
 * anything relying on the fake's ordering fails here rather than in
 * Thunderbird.
 */
export function defaultSeed() {
  const base = Date.UTC(2026, 8, 11, 12, 0, 0);
  const order = [3, 0, 5, 1, 4, 2];
  return order.map((i) => ({
    subject: `Message ${String(i).padStart(2, '0')} about Rechnung ${1000 + i}`,
    author: `Sender ${i} <sender${i}@example.com>`,
    date: base - i * 3_600_000,
    read: i % 2 === 1,
    tags: [],
    headerMessageId: `msg-${i}@example.com`,
    folderId: 'f-inbox',
    body: `Body of message ${i}.`,
  }));
}
