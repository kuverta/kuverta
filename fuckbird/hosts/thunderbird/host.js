/**
 * Thunderbird as a mail host.
 *
 * Implements `core/host.js` over the MailExtension API, and nothing above this
 * file knows it exists. `test/thunderbird-host.test.js` runs the same
 * conformance suite against it that the standalone client's adapter runs.
 *
 * Two things about Thunderbird shape this adapter:
 *
 * **Its lists are cursor-paged and unordered**, where the port asks for an
 * offset into a list sorted newest first. So a scope is materialised once —
 * headers only — sorted here, and cached until something changes it. That is
 * the honest trade: a big folder costs a pause the first time it is opened,
 * and every scroll after that is free. Making it lazy is worth doing when
 * there is a real mailbox to measure it against, and not before.
 *
 * **Moving a message issues a new id.** Archive and trash are both moves, so
 * every id in the cache is stale the moment one lands. The cache is dropped on
 * any change rather than patched, which is also what the triage model expects.
 */

import { MailHost } from '../../core/host.js';
import { ALL_CATEGORIES, CATEGORY_LABELS } from '../../core/category.js';
import { currentCategory, tagKeyFor } from './tags.js';
import { bodyText, factsForMessage } from './messages.js';

/**
 * Most messages a scope will materialise.
 *
 * A ceiling rather than a target. Without one, opening a 200,000-message
 * archive folder would hang the tab with no way to tell what it was doing.
 */
const SCOPE_LIMIT = 20000;

/** Folder names to fall back on, per §3.6, when no SPECIAL-USE says which. */
const SPECIAL_NAMES = {
  archive: ['archive', 'archives', 'archiv', 'alle nachrichten'],
  trash: ['trash', 'deleted items', 'papierkorb', 'gelöschte objekte'],
};

export class ThunderbirdHost extends MailHost {
  #messenger;
  #cache = new Map();
  #undoStack = [];
  #folders = null;

  constructor(messenger) {
    super();
    this.#messenger = messenger;
  }

  get capabilities() {
    return {
      archive: true,
      trash: true,
      setRead: true,
      setCategory: true,
      search: true,
      sync: true,
      // Not `queued`: Thunderbird's write paths do not hold a change back, so
      // by the time undo is offered the move has already happened and another
      // client may have seen it. Undoing is a second change that reverses the
      // first, and the surface says so in those words.
      undo: 'compensating',
      compose: true,
    };
  }

  // -- scopes -------------------------------------------------------------

  async scopes() {
    const folders = await this.#allFolders();
    return [
      { kind: 'all', label: 'Everything' },
      ...folders.map((folder) => ({
        kind: 'folder',
        value: folder,
        label: folder.name ?? folder.path,
      })),
      ...ALL_CATEGORIES.map((category) => ({
        kind: 'category',
        value: category,
        label: CATEGORY_LABELS[category],
      })),
    ];
  }

  // -- listing ------------------------------------------------------------

  async page({ scope, offset, limit }) {
    const rows = await this.#materialise(scope);
    return { total: rows.length, rows: rows.slice(offset, offset + limit) };
  }

  async message(id) {
    const header = await this.#messenger.messages.get(numeric(id));
    if (!header) throw new Error(`no such message: ${id}`);
    const full = await this.#messenger.messages.getFull(numeric(id));
    return {
      row: toRow(header),
      body: bodyText(full),
      facts: await factsForMessage(this.#messenger, numeric(id)),
    };
  }

  // -- acting -------------------------------------------------------------

  async archive(id) {
    const target = await this.#special('archive');
    if (!target) throw new Error('this account has no Archive folder');
    await this.#moveTo(id, target, 'archived');
  }

  async trash(id) {
    const target = await this.#special('trash');
    if (!target) throw new Error('this account has no Trash folder');
    // Never `messages.delete`: that is the one call here that does not move.
    await this.#moveTo(id, target, 'moved to Trash');
  }

  async setRead(id, read) {
    const header = await this.#messenger.messages.get(numeric(id));
    const was = !header.read;
    await this.#messenger.messages.update(numeric(id), { read });
    this.#invalidate();
    this.#undoStack.push({
      what: read ? 'marked read' : 'marked unread',
      undo: async () => {
        await this.#messenger.messages.update(numeric(id), { read: !was });
      },
    });
  }

  async setCategory(id, category) {
    const header = await this.#messenger.messages.get(numeric(id));
    const was = currentCategory(header);
    const theirs = (header.tags ?? []).filter((key) => !ALL_CATEGORIES.some((c) => tagKeyFor(c) === key));

    await this.#messenger.messages.update(numeric(id), {
      tags: [...theirs, tagKeyFor(category)],
    });
    this.#invalidate();

    this.#undoStack.push({
      what: `filed as ${category}`,
      undo: async () => {
        await this.#messenger.messages.update(numeric(id), {
          tags: was === null ? theirs : [...theirs, tagKeyFor(was)],
        });
      },
    });
  }

  async undo() {
    const change = this.#undoStack.pop();
    if (!change) return null;
    await change.undo();
    this.#invalidate();
    return { what: change.what };
  }

  async sync() {
    // Thunderbird syncs on its own schedule and offers no "sync now" to an
    // extension. Saying so is better than a button that silently does nothing.
    this.#invalidate();
    return 'Thunderbird syncs on its own — list refreshed';
  }

  // -- internals ----------------------------------------------------------

  #invalidate() {
    this.#cache.clear();
  }

  /**
   * Every row in a scope, newest first.
   *
   * Cached per scope. The cache is correctness-critical rather than a
   * nicety: without it the offsets the list asks for would be offsets into a
   * differently-ordered list each time, and the cursor would land somewhere
   * new on every keystroke.
   */
  async #materialise(scope) {
    const key = scopeKey(scope);
    if (this.#cache.has(key)) return this.#cache.get(key);

    const headers = await this.#collect(scope);
    const rows = headers
      .map(toRow)
      // Newest first; the id breaks ties so two runs never disagree.
      .sort((a, b) => (b.date ?? 0) - (a.date ?? 0) || a.id.localeCompare(b.id));

    this.#cache.set(key, rows);
    return rows;
  }

  async #collect(scope) {
    const folders =
      scope?.kind === 'folder' ? [scope.value] : await this.#triageFolders();

    const collected = [];
    for (const folder of folders) {
      let page = await this.#messenger.messages.query(this.#queryFor(scope, folder));
      for (;;) {
        collected.push(...page.messages);
        if (collected.length >= SCOPE_LIMIT) {
          console.warn(`fuckbird: stopped listing at ${SCOPE_LIMIT} messages`);
          return collected;
        }
        if (!page.id) break;
        page = await this.#messenger.messages.continueList(page.id);
        if (!page || page.messages.length === 0) break;
      }
    }
    return collected;
  }

  #queryFor(scope, folder) {
    const query = { folderId: folder.id ?? folder };
    switch (scope?.kind) {
      case 'category':
        query.tags = { mode: 'any', tags: { [tagKeyFor(scope.value)]: true } };
        break;
      case 'search':
        query.fullText = String(scope.value);
        break;
      default:
        break;
    }
    return query;
  }

  /**
   * The folders "Everything" means.
   *
   * Not every folder. Querying the whole account would put Archive and Trash
   * in the triage list, so archiving a message would leave it exactly where it
   * was — and on Gmail it would show most mail twice, because `[Gmail]/All
   * Mail` holds a copy of everything (§3.6). Both are the same fix: triage
   * looks at the folders you work from, and nothing else.
   *
   * Gmail's All Mail arrives here as `archives`, so it is excluded by the same
   * rule rather than by a special case.
   */
  async #triageFolders() {
    const excluded = ['archives', 'trash', 'junk', 'sent', 'drafts', 'templates', 'outbox'];
    const folders = await this.#allFolders();
    const kept = folders.filter(
      (folder) => !(folder.specialUse ?? []).some((use) => excluded.includes(use)),
    );

    // A server that advertises no SPECIAL-USE at all would leave everything in
    // that list, including Trash. Fall back to matching the names we know.
    return kept.filter((folder) => {
      const name = (folder.name ?? '').toLowerCase();
      return !SPECIAL_NAMES.archive.includes(name) && !SPECIAL_NAMES.trash.includes(name);
    });
  }

  async #moveTo(id, folder, what) {
    const header = await this.#messenger.messages.get(numeric(id));
    const from = header.folder;
    const messageId = header.headerMessageId;

    await this.#messenger.messages.move([numeric(id)], folder);
    this.#invalidate();

    this.#undoStack.push({
      what,
      undo: async () => {
        // The id is gone with the move, so the message has to be found again
        // where it landed. Its Message-ID is the only handle that survived —
        // and a message may legally have none (§3.6), in which case there is
        // nothing to undo and saying so beats moving the wrong message back.
        if (!messageId) throw new Error('this message has no Message-ID to find it by');
        const found = await this.#messenger.messages.query({
          folderId: folder.id ?? folder,
          headerMessageId: messageId,
        });
        const moved = found.messages?.[0];
        if (!moved) throw new Error('could not find the message to put back');
        await this.#messenger.messages.move([moved.id], from);
      },
    });
  }

  async #allFolders() {
    if (this.#folders !== null) return this.#folders;
    const accounts = await this.#messenger.accounts.list();
    this.#folders = accounts.flatMap((account) => flattenFolders(account.folders ?? []));
    return this.#folders;
  }

  /**
   * The Archive or Trash folder.
   *
   * By `specialUse` where the server said so, and by name where it did not —
   * §3.6: Dovecot advertises SPECIAL-USE but not CREATE-SPECIAL-USE, so a
   * folder created by other software carries no attribute at all.
   */
  async #special(use) {
    const folders = await this.#allFolders();
    const attribute = use === 'archive' ? 'archives' : 'trash';

    const byUse = folders.find((folder) => (folder.specialUse ?? []).includes(attribute));
    if (byUse) return byUse;

    const names = SPECIAL_NAMES[use];
    return (
      folders.find((folder) => names.includes((folder.name ?? '').toLowerCase())) ?? null
    );
  }
}

/** Thunderbird's message ids are numbers; the port's are strings. */
function numeric(id) {
  return typeof id === 'number' ? id : Number(id);
}

function scopeKey(scope) {
  if (!scope) return 'all';
  const value = scope.value?.id ?? scope.value?.path ?? scope.value ?? '';
  return `${scope.kind}:${value}`;
}

function toRow(header) {
  return {
    id: String(header.id),
    subject: header.subject ?? null,
    from: header.author ?? null,
    fromAddr: addressOf(header.author),
    date: header.date ? new Date(header.date).getTime() : null,
    unread: !header.read,
    // Always false, and not an oversight: `MessageHeader` carries no
    // attachment information in any Thunderbird version, so the only ways to
    // know are `listAttachments` per message — thousands of round trips for a
    // paperclip — or a second `query({attachment: true})` per scope, which
    // doubles the cost of the one operation already flagged as this adapter's
    // scaling limit. The list simply does not show the paperclip here. The
    // classifier is unaffected: it reads attachments off the part tree when a
    // message is actually opened.
    hasAttachments: false,
    category: currentCategory(header),
  };
}

function addressOf(author) {
  if (!author) return null;
  const open = author.lastIndexOf('<');
  const close = author.lastIndexOf('>');
  if (open !== -1 && close > open) return author.slice(open + 1, close).trim();
  return author.trim() || null;
}

function flattenFolders(folders) {
  return folders.flatMap((folder) => [folder, ...flattenFolders(folder.subFolders ?? [])]);
}
