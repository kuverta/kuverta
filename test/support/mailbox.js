/**
 * An in-memory mailbox that implements the host port.
 *
 * Two jobs. It is the double the triage model is tested against, and it is the
 * reference implementation that proves the conformance suite is satisfiable at
 * all — a contract no host can pass is a bug in the contract, and without this
 * there would be nothing to notice that with.
 *
 * It is deliberately the *most* capable host: everything declared true, so the
 * suite's capability gates never hide a test from itself.
 */

import { MailHost } from '../../core/host.js';
import { ALL_CATEGORIES, CATEGORY_LABELS } from '../../core/category.js';

const INBOX = 'inbox';
const ARCHIVE = 'archive';
const TRASH = 'trash';

export class MemoryMailbox extends MailHost {
  #messages = new Map();
  #undoStack = [];
  #nextId = 1;

  constructor(seed = defaultSeed()) {
    super();
    for (const message of seed) this.add(message);
  }

  get capabilities() {
    return {
      archive: true,
      trash: true,
      setRead: true,
      setCategory: true,
      search: true,
      sync: true,
      undo: 'compensating',
      compose: true,
    };
  }

  add({ subject, from = 'Someone', fromAddr = 'someone@example.com', date, unread = true, category = null, body = '', hasAttachments = false }) {
    const id = String(this.#nextId++);
    this.#messages.set(id, {
      id,
      subject,
      from,
      fromAddr,
      date,
      unread,
      category,
      hasAttachments,
      body,
      folder: INBOX,
    });
    return id;
  }

  async scopes() {
    return [
      { kind: 'all', label: 'Everything' },
      { kind: 'folder', value: INBOX, label: 'Inbox' },
      ...ALL_CATEGORIES.map((category) => ({
        kind: 'category',
        value: category,
        label: CATEGORY_LABELS[category],
      })),
    ];
  }

  async page({ scope, offset, limit }) {
    const all = this.#matching(scope);
    return {
      total: all.length,
      rows: all.slice(offset, offset + limit).map(toRow),
    };
  }

  async message(id) {
    const message = this.#messages.get(id);
    if (!message) throw new Error(`no such message: ${id}`);
    return { row: toRow(message), body: message.body, facts: null };
  }

  async archive(id) {
    this.#move(id, ARCHIVE, 'archived');
  }

  async trash(id) {
    this.#move(id, TRASH, 'moved to Trash');
  }

  async setRead(id, read) {
    const message = this.#need(id);
    const was = message.unread;
    message.unread = !read;
    this.#undoStack.push({
      what: read ? 'marked read' : 'marked unread',
      undo: () => {
        message.unread = was;
      },
    });
  }

  async setCategory(id, category) {
    const message = this.#need(id);
    const was = message.category;
    message.category = category;
    this.#undoStack.push({
      what: `filed as ${category}`,
      undo: () => {
        message.category = was;
      },
    });
  }

  async undo() {
    const change = this.#undoStack.pop();
    if (!change) return null;
    change.undo();
    return { what: change.what };
  }

  async sync() {
    return 'up to date';
  }

  // -- internals ----------------------------------------------------------

  #need(id) {
    const message = this.#messages.get(id);
    if (!message) throw new Error(`no such message: ${id}`);
    return message;
  }

  #move(id, folder, what) {
    const message = this.#need(id);
    const was = message.folder;
    message.folder = folder;
    this.#undoStack.push({
      what,
      undo: () => {
        message.folder = was;
      },
    });
  }

  #matching(scope) {
    const visible = [...this.#messages.values()].filter(
      (message) => message.folder === INBOX,
    );

    let matched = visible;
    if (scope?.kind === 'folder') {
      matched = [...this.#messages.values()].filter((m) => m.folder === scope.value);
    } else if (scope?.kind === 'category') {
      matched = visible.filter((m) => m.category === scope.value);
    } else if (scope?.kind === 'search') {
      const needle = String(scope.value).toLowerCase();
      matched = visible.filter(
        (m) =>
          (m.subject ?? '').toLowerCase().includes(needle) ||
          (m.body ?? '').toLowerCase().includes(needle),
      );
    }

    // Newest first, ties broken by id so the order never wobbles.
    return [...matched].sort((a, b) => b.date - a.date || Number(b.id) - Number(a.id));
  }
}

function toRow(message) {
  return {
    id: message.id,
    subject: message.subject,
    from: message.from,
    fromAddr: message.fromAddr,
    date: message.date,
    unread: message.unread,
    hasAttachments: message.hasAttachments,
    category: message.category,
  };
}

/** Ten messages, distinct subjects, descending dates. */
export function defaultSeed() {
  const base = Date.UTC(2026, 8, 11, 12, 0, 0);
  return Array.from({ length: 10 }, (_, i) => ({
    subject: `Message ${String(i).padStart(2, '0')} about Rechnung ${1000 + i}`,
    from: `Sender ${i}`,
    fromAddr: `sender${i}@example.com`,
    date: base - i * 3_600_000,
    unread: i % 2 === 0,
    body: `Body of message ${i}.`,
  }));
}
