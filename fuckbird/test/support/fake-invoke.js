/**
 * Enough of `fuckmail`'s Tauri commands to run the host contract against.
 *
 * The shapes are taken from `core-rpc`: `MessageRow` carries `date_utc` in
 * seconds and snake_case keys, `category_counts` answers with pairs rather
 * than an object, `undo` answers `null` for an empty queue, and `messages`
 * takes a folder id rather than a folder name. Getting any of those wrong in
 * the fake would hide exactly the bug it exists to catch.
 *
 * Undo here is `queued` in name only — the fake applies changes immediately
 * and reverses them on undo. What matters for the contract is that undo
 * reverses the last change and that the list agrees afterwards.
 */

const ARCHIVE = 'Archive';
const TRASH = 'Trash';
const INBOX = 'INBOX';

/** The six `core-rules` knows. `set_category` refuses anything else. */
const CATEGORIES = [
  'personal',
  'newsletter',
  'marketing',
  'transactional',
  'notification',
  'unknown',
];

export function fakeInvoke({ seed = defaultSeed(), paper = defaultPaper() } = {}) {
  const { paperMailboxes, documents } = paper;
  const messages = new Map();
  const queue = [];
  let nextId = 1;

  for (const message of seed) {
    const id = nextId++;
    messages.set(id, { ...message, id });
  }

  const need = (id) => {
    const message = messages.get(Number(id));
    if (!message) throw new Error(`no such message: ${id}`);
    return message;
  };

  const visible = () =>
    [...messages.values()]
      .filter((m) => m.folder === INBOX)
      // The store sorts; so does the fake, newest first.
      .sort((a, b) => b.date_utc - a.date_utc || b.id - a.id);

  const asRow = (m) => ({
    id: m.id,
    date_utc: m.date_utc,
    from: m.from,
    subject: m.subject,
    unread: m.unread,
    has_attachments: m.has_attachments,
    list_id: m.list_id ?? null,
    category: m.category ?? null,
    snippet: m.snippet ?? null,
  });

  const commands = {
    special_folders: async () => ({ archive: ARCHIVE, trash: TRASH }),

    folders: async () => [
      { id: 1, name: INBOX, label: 'Inbox', special_use: 'inbox', total: messages.size, unread: 0 },
      { id: 2, name: ARCHIVE, label: 'Archive', special_use: 'archive', total: 0, unread: 0 },
    ],

    category_counts: async () => {
      const counts = new Map();
      for (const m of visible()) {
        if (m.category) counts.set(m.category, (counts.get(m.category) ?? 0) + 1);
      }
      return [...counts];
    },

    messages: async ({ offset, limit, category, folder }) => {
      let rows = visible();
      if (category) rows = rows.filter((m) => m.category === category);
      if (folder) {
        const name = folder === 2 ? ARCHIVE : INBOX;
        rows = [...messages.values()]
          .filter((m) => m.folder === name)
          .sort((a, b) => b.date_utc - a.date_utc || b.id - a.id);
      }
      return { total: rows.length, offset, rows: rows.slice(offset, offset + limit).map(asRow) };
    },

    search: async ({ query, limit }) => {
      const needle = String(query).toLowerCase();
      return visible()
        .filter(
          (m) =>
            (m.subject ?? '').toLowerCase().includes(needle) ||
            (m.snippet ?? '').toLowerCase().includes(needle),
        )
        .slice(0, limit)
        .map(asRow);
    },

    message: async ({ id }) => {
      const m = need(id);
      return {
        id: m.id,
        message_id: m.message_id ?? null,
        subject: m.subject,
        from: m.from,
        date_utc: m.date_utc,
        to: [],
        cc: [],
        folders: [m.folder],
        body_text: m.body ?? null,
        // The shape core-rpc sends: the classifier's facts, camelCase.
        facts: {
          fromAddr: m.from?.match(/<([^>]+)>/)?.[1] ?? m.from ?? null,
          fromName: m.from?.split(' <')[0] ?? null,
          subject: m.subject ?? null,
          listId: m.list_id ?? null,
          listUnsubscribe: null,
          precedence: null,
          autoSubmitted: null,
          inReplyTo: null,
          hasAttachments: Boolean(m.has_attachments),
          recipientCount: 1,
          snippet: m.snippet ?? null,
        },
      };
    },

    move_to: async ({ id, target }) => {
      const m = need(id);
      const was = m.folder;
      m.folder = target;
      queue.push({ what: `move to ${target}`, undo: () => { m.folder = was; } });
      return m.id;
    },

    set_read: async ({ id, read }) => {
      const m = need(id);
      const was = m.unread;
      m.unread = !read;
      queue.push({ what: read ? 'mark read' : 'mark unread', undo: () => { m.unread = was; } });
      return m.id;
    },

    set_category: async ({ id, category }) => {
      // core-rpc validates against core-rules and refuses anything else; a
      // fake that accepted any string would hide an adapter sending one.
      if (!CATEGORIES.includes(category)) throw new Error(`not a category: ${category}`);
      const m = need(id);
      // Not queued, and so not undoable — matching the real command.
      m.category = category;
    },

    // -- post ------------------------------------------------------------

    paper_mailboxes: async () => paperMailboxes,

    paper_documents: async ({ id, offset, limit, query }) => {
      const box = paperMailboxes.find((m) => m.id === id);
      if (!box) throw new Error(`no postal address ${id}`);
      let rows = documents.filter((d) => d.mailbox === id);
      if (query) {
        const needle = String(query).toLowerCase();
        rows = rows.filter((d) => d.subject.toLowerCase().includes(needle));
      }
      rows = [...rows].sort((a, b) => b.date_utc - a.date_utc);
      return { total: rows.length, offset, rows: rows.slice(offset, offset + limit) };
    },

    file_post: async ({ id, documentId, category }) => {
      // The real command validates against core-rules and asks Paperless who
      // sent the letter; the fake keeps the part a test can observe.
      if (!CATEGORIES.includes(category)) throw new Error(`not a category: ${category}`);
      const found = documents.find((d) => d.mailbox === id && d.id === documentId);
      if (!found) throw new Error(`no such document: ${documentId}`);
      found.category = category;
    },

    paper_document: async ({ id, documentId }) => {
      const found = documents.find((d) => d.mailbox === id && d.id === documentId);
      if (!found) throw new Error(`no such document: ${documentId}`);
      return {
        row: found,
        body_text: found.snippet,
        download_url: 'http://x/1/download/',
        correspondent: found.from,
        // As core-paper builds them: the correspondent is the address.
        facts: {
          fromAddr: found.from,
          fromName: found.from,
          subject: found.subject,
          listId: null,
          listUnsubscribe: null,
          precedence: null,
          autoSubmitted: null,
          inReplyTo: null,
          hasAttachments: true,
          recipientCount: 1,
          snippet: found.snippet,
        },
      };
    },

    undo: async () => {
      const change = queue.pop();
      if (!change) return null;
      change.undo();
      return { id: 1, message_id: 1, what: change.what, holds_for: 0, last_error: null };
    },

    sync: async () => ({
      email: 'you@example.com',
      changes_sent: queue.length,
      changes_obsolete: 0,
      changes_refused: 0,
      changes_retryable: 0,
      non_atomic_moves: 0,
      folders_synced: 1,
      folders_skipped: 0,
      folders_excluded: 0,
      inserted: 0,
      deduplicated: 0,
      flag_updates: 0,
      expunged: 0,
    }),
  };

  const invoke = async (command, args = {}) => {
    const handler = commands[command];
    // Tauri throws for a command that was never registered, and so must this:
    // an adapter calling a command that does not exist is a bug to find here.
    if (!handler) throw new Error(`unknown command: ${command}`);
    return handler(args);
  };

  invoke._messages = messages;
  return invoke;
}

export function defaultSeed() {
  const base = Math.floor(Date.UTC(2026, 8, 11, 12, 0, 0) / 1000);
  return [3, 0, 5, 1, 4, 2].map((i) => ({
    subject: `Message ${String(i).padStart(2, '0')} about Rechnung ${1000 + i}`,
    from: `Sender ${i} <sender${i}@example.com>`,
    date_utc: base - i * 3600,
    unread: i % 2 === 0,
    has_attachments: false,
    category: null,
    folder: 'INBOX',
    message_id: `msg-${i}@example.com`,
    snippet: `Body of message ${i}.`,
    body: `Body of message ${i}.`,
  }));
}

/**
 * One physical address with two pieces of post.
 *
 * Dated between the seeded messages on purpose: if post only sorted correctly
 * when it was all newer or all older than the mail, the merge would look right
 * and be wrong.
 */
export function defaultPaper() {
  const base = Math.floor(Date.UTC(2026, 8, 11, 12, 0, 0) / 1000);
  return {
    paperMailboxes: [
      {
        id: 1,
        label: 'Home',
        base_url: 'http://localhost:8000',
        selector_kind: 'everything',
        selector_value: null,
        has_token: true,
      },
    ],
    documents: [
      {
        id: 41,
        mailbox: 1,
        date_utc: base - 1800,
        from: 'Stadtwerke München',
        subject: 'Ihre Abschlagszahlung für April',
        unread: false,
        has_attachments: true,
        category: 'transactional',
        snippet: 'Ihr monatlicher Abschlag beträgt ab April 84,00 EUR.',
        tags: ['home'],
        page_count: 2,
      },
      {
        id: 40,
        mailbox: 1,
        date_utc: base - 9000,
        from: 'Finanzamt',
        subject: 'Bescheid über Einkommensteuer',
        unread: false,
        has_attachments: true,
        category: 'transactional',
        snippet: 'Ihr Steuerbescheid liegt bei.',
        tags: [],
        page_count: 4,
      },
    ],
  };
}
