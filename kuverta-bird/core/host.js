/**
 * The seam between the triage surface and whatever is holding the mail.
 *
 * There are two hosts: Thunderbird, where this runs as a MailExtension, and
 * `kuverta`, where it runs in the desktop app's webview. Everything above this
 * line — the classifier, the list, the cursor, the key map — is written once
 * and knows about neither. Everything below it is a few hundred lines per host.
 *
 * The rule that keeps that true: **the core never sees a host type.** Message
 * ids are opaque strings, folders are opaque handles, and anything a host can
 * do that the other cannot is declared in `capabilities` rather than sniffed
 * for. A core that starts asking "am I in Thunderbird?" has stopped being a
 * core.
 *
 * `test/conformance.js` is the contract. An adapter that passes it can drive
 * the triage surface; one that does not will fail in ways the surface cannot
 * sensibly handle.
 */

/**
 * @typedef {object} Row  One line in the list. Everything needed to draw it and
 *   nothing more — bodies are fetched only when a message is opened.
 * @property {string} id       Opaque to the core, and shorter-lived than it
 *   looks: a host may issue a new id for a message that has *moved*, which
 *   archiving and trashing both do. Thunderbird does exactly this. So an id is
 *   good until the message is acted on, and no longer — which is why the
 *   triage model reloads the list after every action rather than patching the
 *   row it already has. Nothing may hold an id across an action and expect it
 *   to still resolve.
 * @property {?string} subject
 * @property {?string} from    Display name if there is one, else the address.
 * @property {?string} fromAddr
 * @property {?number} date    Epoch milliseconds.
 * @property {boolean} unread
 * @property {boolean} hasAttachments
 * @property {?string} category  As filed; null if this message was never classified.
 * @property {string[]} [actions]  Which of `ACTIONS` this row allows, when it
 *   allows less than its host can do. Absent means "everything the host can
 *   do", which is the case for mail.
 *
 *   This exists because a list can hold more than one kind of thing. Post read
 *   from Paperless sits in the same list as mail and cannot be archived,
 *   trashed or marked read — Paperless owns those documents. Capabilities
 *   answer "what can this host do"; this answers "what can be done to this
 *   row", and a mixed list needs both.
 */

/**
 * @typedef {object} Page
 * @property {Row[]} rows
 * @property {number} total  How many messages the scope holds, or -1 when the
 *   host cannot say cheaply. The list copes with -1; it just cannot show a
 *   scrollbar of the right length.
 */

/**
 * What the list is showing.
 *
 * Kept as data rather than as a folder handle because "everything the
 * classifier called a newsletter" is a first-class way to look at a mailbox
 * here, and it does not correspond to a folder in either host.
 *
 * @typedef {object} Scope
 * @property {'folder'|'category'|'search'|'all'} kind
 * @property {*} [value]  A folder handle, a category name, or a query string.
 * @property {string} label  What to show the user. Never empty: a filtered list
 *   that does not say what it is filtering looks exactly like mail going
 *   missing.
 */

/**
 * What a host can do.
 *
 * Declared, not detected. The surface hides the keys for things the host cannot
 * do rather than offering them and failing.
 *
 * @typedef {object} Capabilities
 * @property {boolean} archive
 * @property {boolean} trash
 * @property {boolean} setRead
 * @property {boolean} setCategory
 * @property {boolean} search
 * @property {boolean} sync     Whether a manual sync means anything.
 * @property {'queued'|'compensating'|false} undo  See below.
 * @property {boolean} compose
 */

/**
 * The two kinds of undo, which are not the same promise and must not look the
 * same to the user.
 *
 * `queued` — the change has not left the machine yet. Undo cancels it, and
 *   nothing ever happened. This is what `kuverta` offers, per brief §3.5: "a
 *   change stays cancellable for as long as it is queued".
 *
 * `compensating` — the change has already happened and undo performs its
 *   inverse: the message moves back. Honest, and much weaker. Another client
 *   may have seen the intermediate state, the inverse can fail on its own, and
 *   for anything genuinely irreversible there is no inverse to perform. This is
 *   what Thunderbird can offer, because its write paths do not queue.
 *
 * The surface says which it has. "Undone" and "cancelled" are different words
 * for a reason.
 */

/** Actions the list can take on the message under the cursor. */
export const ACTIONS = Object.freeze({
  Archive: 'archive',
  Trash: 'trash',
  ToggleRead: 'toggleRead',
  SetCategory: 'setCategory',
  Open: 'open',
});

/**
 * Base class, and the documentation of the contract.
 *
 * Subclassing is optional — anything with these methods will do — but
 * inheriting gives you the "this host cannot do that" errors for free, which
 * is better than an adapter silently doing nothing.
 */
export class MailHost {
  /** @returns {Capabilities} */
  get capabilities() {
    return {
      archive: false,
      trash: false,
      setRead: false,
      setCategory: false,
      search: false,
      sync: false,
      undo: false,
      compose: false,
    };
  }

  /**
   * The scopes to offer in the sidebar, in the order to offer them.
   *
   * @returns {Promise<Scope[]>}
   */
  async scopes() {
    return [];
  }

  /**
   * One page of the list.
   *
   * Paged because a mailbox does not fit in memory and the list is windowed.
   * A host that cannot page may return everything and a real `total`.
   *
   * @param {{scope: Scope, offset: number, limit: number}} request
   * @returns {Promise<Page>}
   */
  async page() {
    throw new Unsupported('page');
  }

  /**
   * Everything needed to read one message.
   *
   * @param {string} id
   * @returns {Promise<{row: Row, body: ?string, facts: ?object}>}
   */
  async message() {
    throw new Unsupported('message');
  }

  /** @param {string} id */
  async archive() {
    throw new Unsupported('archive');
  }

  /** @param {string} id */
  async trash() {
    throw new Unsupported('trash');
  }

  /**
   * @param {string} id
   * @param {boolean} read
   */
  async setRead() {
    throw new Unsupported('setRead');
  }

  /**
   * Files a message under a category, and records the correction.
   *
   * The host owns this rather than the core because where a correction is
   * stored differs — extension storage in one, a table in the other — and
   * because the host is what knows how to show the category on a message.
   *
   * @param {string} id
   * @param {string} category
   */
  async setCategory() {
    throw new Unsupported('setCategory');
  }

  /**
   * Reverses the last change.
   *
   * @returns {Promise<?{what: string}>} What was undone, or null if there was
   *   nothing to undo. Never throws for an empty stack — an empty undo is a
   *   normal thing to ask for.
   */
  async undo() {
    throw new Unsupported('undo');
  }

  /** @returns {Promise<?string>} A line to show, or null. */
  async sync() {
    throw new Unsupported('sync');
  }
}

/**
 * Raised when the surface asks for something `capabilities` said was not there.
 *
 * A distinct type so the surface can say "this host cannot archive" rather than
 * showing whatever a host's internal error happened to look like.
 */
export class Unsupported extends Error {
  constructor(what) {
    super(`this host cannot ${what}`);
    this.name = 'Unsupported';
    this.what = what;
  }
}

/**
 * A scope label that is always safe to show.
 *
 * §3.1: a filtered list says what it is showing. This is the last line of
 * defence for a host that returned a scope with no label.
 */
export function describeScope(scope) {
  if (!scope) return 'Everything';
  if (scope.label) return scope.label;
  switch (scope.kind) {
    case 'category':
      return `Category: ${scope.value}`;
    case 'search':
      return `Search: ${scope.value}`;
    case 'folder':
      return 'Folder';
    default:
      return 'Everything';
  }
}
