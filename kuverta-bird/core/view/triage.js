/**
 * The triage surface.
 *
 * Mounted by both hosts against a `Triage` over their own adapter, so the list
 * looks and behaves the same in the Thunderbird tab and in the standalone
 * client's window. It knows about the DOM and about `core/triage.js`, and
 * about neither Thunderbird nor `kuverta`.
 *
 * The list is windowed: a screenful of rows exists in the DOM and a spacer
 * gives the scroller the height of the whole mailbox. A mailbox has more
 * messages than a browser will happily lay out, and finding that out at
 * 40,000 messages is worse than building it this way now.
 */

import { CATEGORY_LABELS, CATEGORY_MEANINGS, ALL_CATEGORIES } from '../category.js';
import { Classifier } from '../classify.js';
import { CHEAT_SHEET, INTENT, INTENT_ACTIONS, PRIMARY_KEYS, intentFor, isTyping } from '../keys.js';

const ROW_HEIGHT = 56;
/** Where "you have been shown the explanation" is remembered. */
const SEEN_KEY = 'kuverta-bird.explained';
/** Rows drawn above and below the visible window, so scrolling is not blank. */
const OVERSCAN = 6;

/**
 * @param {object} options
 * @param {HTMLElement} options.root   Where to build the surface.
 * @param {import('../triage.js').Triage} options.triage
 * @param {Function} [options.onCompose]  Host-specific; hidden when absent.
 * @param {string} [options.title]  What this mailbox is — an account address,
 *   usually. The core has no idea which account it is looking at, and the
 *   person reading it very much needs to.
 */
export function mountTriage({ root, triage, onCompose = null, title = '' }) {
  root.classList.add('fb-triage');
  root.innerHTML = LAYOUT;

  const el = (id) => root.querySelector(`#fb-${id}`);
  const scopeLabel = el('scope-label');
  const scopeOut = el('scope-out');
  const emptyList = el('empty-list');
  const sidebar = el('sidebar');
  const viewport = el('viewport');
  const spacer = el('spacer');
  const content = el('content');
  const reading = el('reading');
  const why = el('why');
  const empty = el('empty');
  const toast = el('toast');
  const help = el('help');
  const search = el('search');
  const legend = el('legend');

  /** Reused row elements. Rebuilt only when the window is resized. */
  let pool = [];
  /** The sidebar's scopes, kept so the panel can quote this mailbox's own numbers. */
  let scopes = [];
  let firstRendered = -1;
  let lastNotice = 0;

  el('title').textContent = title;
  buildHelp();
  buildLegend();
  buildPool();
  drawSidebar();

  const unsubscribe = triage.subscribe(() => {
    draw(true);
    drawNotice();
    drawReading();
  });

  viewport.addEventListener('scroll', () => draw(false), { passive: true });

  // Everything else is attached to elements inside `root`, which the next
  // mount replaces wholesale. These two outlive it, so they are the two that
  // have to be given back.
  const onResize = () => {
    buildPool();
    draw(true);
  };
  window.addEventListener('resize', onResize);

  content.addEventListener('click', (event) => {
    const row = event.target.closest('.fb-row');
    if (!row) return;
    triage.select(Number(row.dataset.index));
    triage.openSelected();
  });

  scopeOut.addEventListener('click', () => triage.clearScope());

  el('helpbtn').addEventListener('click', () => {
    if (help.hidden) help.hidden = false;
    else closeHelp();
  });
  // Only the backdrop dismisses it. Making the whole overlay dismiss on click
  // would eat every click meant for the text inside it.
  help.addEventListener('click', (event) => {
    if (event.target === help) closeHelp();
  });

  search.addEventListener('keydown', (event) => {
    if (event.key === 'Enter') {
      const query = search.value.trim();
      if (query) {
        triage.setScope({ kind: 'search', value: query, label: `Search: ${query}` });
      } else {
        triage.clearScope();
      }
      search.blur();
    }
    if (event.key === 'Escape') {
      search.value = '';
      search.blur();
      triage.clearScope();
    }
  });

  document.addEventListener('keydown', onKey);

  draw(true);

  // First launch: say what this is before anyone has to ask. This line was
  // meant to exist from the start and did not — a patch that should have added
  // it matched nothing, silently — so the person it was written for never saw
  // the panel unless they pressed `?`. test/view.test.js now fails without it.
  if (!seenBefore()) help.hidden = false;

  /**
   * Lets go of the document.
   *
   * A host that can switch mailboxes mounts this more than once. Without a
   * teardown the old surface keeps listening, and every keystroke acts twice —
   * once on a list nobody can see, which with `e` and `#` bound means archiving
   * a message the user never looked at.
   */
  return () => {
    document.removeEventListener('keydown', onKey);
    window.removeEventListener('resize', onResize);
    unsubscribe();
  };

  // -- keys ---------------------------------------------------------------

  async function onKey(event) {
    // While the user is typing, every key belongs to the field. Without this,
    // typing `e` into the search box archives something.
    if (isTyping(event.target)) return;

    if (!help.hidden && event.key === 'Escape') {
      closeHelp();
      return;
    }

    // Digits file by category: 1 is the first category, and so on. Only
    // offered when the host can actually do it.
    if (/^[1-9]$/.test(event.key) && triage.capabilities.setCategory) {
      const category = ALL_CATEGORIES[Number(event.key) - 1];
      if (category) {
        event.preventDefault();
        await triage.setCategory(category);
      }
      return;
    }

    const intent = intentFor(event);
    if (intent === null) return;

    const action = INTENT_ACTIONS[intent];
    if (action) {
      event.preventDefault();
      await triage.act(action);
      return;
    }

    switch (intent) {
      case INTENT.Down:
        event.preventDefault();
        triage.move(1);
        break;
      case INTENT.Up:
        event.preventDefault();
        triage.move(-1);
        break;
      case INTENT.Undo:
        event.preventDefault();
        await triage.undo();
        break;
      case INTENT.Sync:
        event.preventDefault();
        await triage.sync();
        break;
      case INTENT.Search:
        event.preventDefault();
        search.focus();
        search.select();
        break;
      case INTENT.Close:
        if (triage.open) triage.closeOpen();
        else if (triage.isFiltered) triage.clearScope();
        break;
      case INTENT.Explain:
        event.preventDefault();
        if (help.hidden) help.hidden = false;
        else closeHelp();
        break;
      case INTENT.Compose:
        if (onCompose) {
          event.preventDefault();
          onCompose();
        }
        break;
      default:
        break;
    }
  }

  // -- drawing ------------------------------------------------------------

  function buildPool() {
    const visible = Math.ceil(viewport.clientHeight / ROW_HEIGHT) || 10;
    const size = visible + OVERSCAN * 2;
    content.replaceChildren();
    pool = Array.from({ length: size }, () => {
      const node = document.createElement('div');
      node.className = 'fb-row';
      node.innerHTML = ROW;
      content.append(node);
      return node;
    });
    firstRendered = -1;
  }

  function draw(force) {
    spacer.style.height = `${Math.max(triage.total, 0) * ROW_HEIGHT}px`;

    const first = Math.max(0, Math.floor(viewport.scrollTop / ROW_HEIGHT) - OVERSCAN);
    if (!force && first === firstRendered) return;
    firstRendered = first;
    content.style.transform = `translateY(${first * ROW_HEIGHT}px)`;

    pool.forEach((node, i) => {
      const index = first + i;
      const row = triage.rowAt(index);

      if (index >= triage.total) {
        node.hidden = true;
        return;
      }
      node.hidden = false;
      node.dataset.index = String(index);
      node.classList.toggle('fb-selected', index === triage.selected);

      if (!row) {
        // Not fetched yet. Asking here is what makes the list load as it
        // scrolls rather than all at once.
        node.classList.add('fb-pending');
        node.querySelector('.fb-subject').textContent = '…';
        node.querySelector('.fb-from').textContent = '';
        node.querySelector('.fb-date').textContent = '';
        node.querySelector('.fb-cat').textContent = '';
        triage.ensureLoaded(index);
        return;
      }

      node.classList.remove('fb-pending');
      node.classList.toggle('fb-unread', row.unread);
      node.querySelector('.fb-subject').textContent = row.subject || '(no subject)';
      node.querySelector('.fb-from').textContent = row.from || row.fromAddr || '';
      node.querySelector('.fb-date').textContent = when(row.date);

      const category = node.querySelector('.fb-cat');
      category.textContent = row.category ? CATEGORY_LABELS[row.category] : '';
      category.dataset.category = row.category ?? '';

      node.querySelector('.fb-clip').hidden = !row.hasAttachments;
    });

    keepCursorVisible();
    drawScope();
  }

  function keepCursorVisible() {
    if (triage.selected < 0) return;
    const top = triage.selected * ROW_HEIGHT;
    const bottom = top + ROW_HEIGHT;
    if (top < viewport.scrollTop) viewport.scrollTop = top;
    else if (bottom > viewport.scrollTop + viewport.clientHeight) {
      viewport.scrollTop = bottom - viewport.clientHeight;
    }
  }

  function drawScope() {
    // Always shown, never hidden. §3.1 asks a filtered list to say what it is
    // showing; a list that says nothing when unfiltered leaves you working out
    // where you are from the rows, which is the same problem in a milder form.
    const count = triage.total === 1 ? '1 message' : `${triage.total} messages`;
    scopeLabel.textContent = `${triage.scopeLabel} · ${count}`;

    // The way out appears only when there is something to get out of, so an
    // unfiltered list does not carry a button that does nothing.
    scopeOut.hidden = !triage.isFiltered;

    // An empty list has to say why it is empty and what to do about it.
    // Otherwise it is indistinguishable from mail having gone missing.
    emptyList.hidden = triage.total !== 0;
    if (triage.total === 0) {
      emptyList.textContent = triage.isFiltered
        ? `Nothing in ${triage.scopeLabel}. Press Escape, or use “Show everything”.`
        : 'No messages here yet. Sync with r, or pick a folder on the left.';
    }
  }

  function drawSidebar() {
    triage.host.scopes().then((loaded) => {
      scopes = loaded;
      // Rebuilt now that there are real counts to put in it. "33 of these were
      // written by a person" says what this is for; a paragraph about
      // categories only says what it does.
      buildHelp();
      sidebar.replaceChildren(
        ...scopes.map((scope) => {
          const button = document.createElement('button');
          button.className = 'fb-scope';
          button.textContent = scope.label;
          if (scope.kind === 'category') {
            button.title = CATEGORY_MEANINGS[scope.value] ?? '';
          }
          if (scope.count) {
            const count = document.createElement('span');
            count.className = 'fb-count';
            count.textContent = String(scope.count);
            button.append(count);
          }
          button.addEventListener('click', () => triage.setScope(scope));
          return button;
        }),
      );
    });
  }

  function drawReading() {
    const open = triage.open;
    reading.hidden = !open;
    empty.hidden = Boolean(open);
    if (!open) {
      // A highlighted row beside a pane saying "select a message" reads as
      // though the selection did not take. Say what to press instead.
      empty.textContent = triage.selectedRow
        ? 'Press ↵ to read this message'
        : 'Nothing selected';
      return;
    }

    reading.querySelector('.fb-read-subject').textContent = open.row.subject || '(no subject)';
    reading.querySelector('.fb-read-meta').textContent =
      `${open.row.from ?? ''} · ${when(open.row.date)}`;
    // textContent, never innerHTML: this is a mail body, which is to say it is
    // written by whoever sent it.
    reading.querySelector('.fb-read-body').textContent =
      open.body ?? '(no plain-text body)';

    drawWhy(open);
  }

  /**
   * Why this message is where it is.
   *
   * The explanation is the point of a rules layer — a verdict you cannot argue
   * with is one you learn to ignore. The reasons are recomputed here from the
   * facts the host hands over rather than stored, because they are a pure
   * function of those facts and a cached explanation can go stale against the
   * verdict it is explaining.
   *
   * A host that cannot supply facts still gets the category and what it means,
   * which is a weaker answer to the same question rather than no answer.
   */
  function drawWhy(open) {
    const filed = open.row.category ?? null;
    why.replaceChildren();
    if (!filed && !open.facts) {
      why.hidden = true;
      return;
    }
    why.hidden = false;

    const head = document.createElement('div');
    head.className = 'fb-why-head';
    const pill = document.createElement('span');
    pill.className = 'fb-pill';
    pill.dataset.category = filed ?? 'unknown';
    pill.textContent = filed ? CATEGORY_LABELS[filed] : 'Not filed';
    head.append(pill);
    why.append(head);

    if (!open.facts) {
      // No facts from this host, so say what the category means and stop.
      // Guessing at reasons would be worse than admitting to having none.
      why.append(paragraph(filed ? CATEGORY_MEANINGS[filed] : ''));
      return;
    }

    // Deliberately without the corrections history: this is what the rules
    // alone make of the message, which is the thing worth showing beside a
    // verdict that may have come from a correction instead.
    const verdict = Classifier.withoutHistory().classify(open.facts);

    if (filed && verdict.category !== filed) {
      // The two disagree, which on this path means the filing came from you.
      why.append(
        paragraph(
          `You filed this as ${CATEGORY_LABELS[filed]}. On the headers alone the ` +
            `rules would have said ${CATEGORY_LABELS[verdict.category]}.`,
        ),
      );
      return;
    }

    if (verdict.reasons.length === 0) {
      why.append(paragraph(CATEGORY_MEANINGS[verdict.category]));
      return;
    }

    const list = document.createElement('ul');
    for (const reason of verdict.reasons) {
      const item = document.createElement('li');
      item.textContent = reason.detail;
      list.append(item);
    }
    why.append(list);
  }

  function drawNotice() {
    const notice = triage.notice;
    if (!notice || notice.at === lastNotice) return;
    lastNotice = notice.at;
    toast.textContent = notice.text;
    toast.className = `fb-toast${notice.tone === 'error' ? ' fb-error' : ''}`;
    toast.hidden = false;
    clearTimeout(toast._timer);
    toast._timer = setTimeout(() => {
      toast.hidden = true;
    }, 4000);
  }

  function buildLegend() {
    legend.replaceChildren(
      ...PRIMARY_KEYS.map(([keys, what]) => {
        const item = document.createElement('span');
        const kbd = document.createElement('kbd');
        kbd.textContent = keys;
        const text = document.createElement('span');
        text.textContent = what;
        item.append(kbd, text);
        return item;
      }),
    );
  }

  /**
   * What this is, before how to drive it.
   *
   * Asked for by the first person to use it, who could work out that the keys
   * did things and not what the thing was. A list of shortcuts answers the
   * second question and silently assumes the first has been answered
   * somewhere — and the only somewhere was a readme in a different repository.
   */
  function buildHelp() {
    const panel = document.createElement('div');
    panel.className = 'fb-help-panel';

    panel.append(
      heading('What this is for'),
      paragraph(
        'Most of a mailbox was not written to you. Newsletters, receipts, alerts and ' +
          'adverts arrive in the same list as the handful of messages that actually ' +
          'need an answer, and in an inbox they all look equally like work.',
      ),
    );

    // The mailbox's own numbers, when the host can count. Far more convincing
    // than the sentence above, and it is the same point.
    const split = mailboxSplit();
    if (split) {
      panel.append(
        paragraph(
          `In this mailbox: ${split.personal} of ${split.total} messages were written ` +
            `by a person. The other ${split.rest} were not.`,
        ),
      );
    }

    panel.append(
      paragraph(
        'Triage sorts that out before you look. Each message is filed under one of six ' +
          'categories, so you can deal with a whole kind at once — read the newsletters ' +
          'when you want to read, do the invoices when you are doing money, and answer ' +
          'the few messages a person actually sent you. That is the point: a mailbox ' +
          'you work through rather than one you read.',
      ),
      paragraph(
        'Nothing is moved and no folder changes. The category is a label, and the list ' +
          'is sorted by it instead of by whichever folder the mail happened to land in.',
      ),
      heading('The six'),
    );

    const table = document.createElement('dl');
    table.className = 'fb-meanings';
    for (const category of ALL_CATEGORIES) {
      const term = document.createElement('dt');
      const pill = document.createElement('span');
      pill.className = 'fb-pill';
      pill.dataset.category = category;
      pill.textContent = CATEGORY_LABELS[category];
      term.append(pill);

      const said = document.createElement('dd');
      said.textContent = CATEGORY_MEANINGS[category];
      table.append(term, said);
    }
    panel.append(table);

    panel.append(
      heading('When it gets one wrong'),
      paragraph(
        'Press 1 to 6 to file a message yourself. That is not just a relabelling: the ' +
          'sender or the list is remembered, so the next message like it is filed the ' +
          'same way without being asked. Corrections are the one signal here that is ' +
          'definitely right, so they override the rules outright — and the reading pane ' +
          'shows why anything was filed where it was, so a wrong answer is arguable ' +
          'rather than just annoying.',
      ),
      heading('Keys'),
    );

    const keys = document.createElement('div');
    keys.className = 'fb-keys';
    for (const [binding, what] of CHEAT_SHEET) {
      const kbd = document.createElement('kbd');
      kbd.textContent = binding;
      const text = document.createElement('span');
      text.textContent = what;
      keys.append(kbd, text);
    }
    panel.append(keys);

    const done = document.createElement('button');
    done.className = 'fb-ghost fb-help-done';
    done.textContent = 'Got it';
    done.addEventListener('click', () => closeHelp());
    panel.append(done);

    help.replaceChildren(panel);
  }

  /**
   * How much of this mailbox was written by a person.
   *
   * Null when the host cannot count cheaply — Thunderbird's scopes carry no
   * totals — in which case the panel makes the point in words alone rather
   * than inventing a number.
   */
  function mailboxSplit() {
    const counted = scopes.filter(
      (scope) => scope.kind === 'category' && typeof scope.count === 'number',
    );
    if (counted.length === 0) return null;

    const total = counted.reduce((sum, scope) => sum + scope.count, 0);
    if (total === 0) return null;

    const personal = counted.find((scope) => scope.value === 'personal')?.count ?? 0;
    return { personal, total, rest: total - personal };
  }

  function heading(text) {
    const h = document.createElement('h2');
    h.textContent = text;
    return h;
  }

  function paragraph(text) {
    const p = document.createElement('p');
    p.textContent = text;
    return p;
  }

  function closeHelp() {
    help.hidden = true;
    // Remembered so it explains itself once and then gets out of the way.
    // Wrapped because storage can be unavailable or full, and failing to
    // record that the panel was dismissed is not worth taking the surface
    // down for — it just means it opens again next time.
    try {
      localStorage.setItem(SEEN_KEY, '1');
    } catch {
      /* not worth caring about */
    }
  }

  function seenBefore() {
    try {
      return localStorage.getItem(SEEN_KEY) === '1';
    } catch {
      return false;
    }
  }
}

/** A date a reader can scan: a time today, a weekday this week, else a date. */
function when(millis) {
  if (!millis) return '';
  const date = new Date(millis);
  const now = new Date();
  const sameDay = date.toDateString() === now.toDateString();
  if (sameDay) return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  const days = (now - date) / 86_400_000;
  if (days < 7) return date.toLocaleDateString([], { weekday: 'short' });
  return date.toLocaleDateString([], { day: '2-digit', month: 'short' });
}

const ROW = `
  <span class="fb-cat"></span>
  <span class="fb-from"></span>
  <span class="fb-subject"></span>
  <span class="fb-clip" hidden>📎</span>
  <span class="fb-date"></span>
`;

const LAYOUT = `
  <aside id="fb-sidebar" class="fb-sidebar"></aside>
  <section class="fb-main">
    <div class="fb-head">
      <span id="fb-title" class="fb-title"></span>
      <input id="fb-search" class="fb-search" type="search" placeholder="/ to search" spellcheck="false">
      <button id="fb-helpbtn" class="fb-ghost" title="Shortcuts (?)">?</button>
    </div>
    <div class="fb-scopebar">
      <span id="fb-scope-label"></span>
      <button id="fb-scope-out" class="fb-ghost" hidden>Show everything</button>
    </div>
    <div id="fb-viewport" class="fb-viewport">
      <div id="fb-spacer"></div>
      <div id="fb-content"></div>
      <p id="fb-empty-list" class="fb-empty-list fb-muted" hidden></p>
    </div>
  </section>
  <section class="fb-detail">
    <div id="fb-empty" class="fb-muted">Nothing selected</div>
    <article id="fb-reading" hidden>
      <h2 class="fb-read-subject"></h2>
      <p class="fb-read-meta fb-muted"></p>
      <pre class="fb-read-body"></pre>
      <aside id="fb-why" class="fb-why" hidden></aside>
    </article>
  </section>
  <footer id="fb-legend" class="fb-legend"></footer>
  <div id="fb-toast" class="fb-toast" hidden></div>
  <div id="fb-help" class="fb-help" hidden></div>
`;
