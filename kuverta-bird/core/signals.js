/**
 * The individual heuristics.
 *
 * Weights are on a rough scale where 3.0 is "this header exists for exactly
 * this purpose", 1.5 is "a good hint", and 0.75 is "worth a nudge". They are
 * guesses, and they are meant to be: the point of recording every verdict
 * alongside the model's is to replace guesses with measurements.
 *
 * Keywords are German and English because that is what the mail is.
 *
 * Ported from `kuverta`'s `core-rules/src/signals.rs`. Keep the rule ids
 * identical to the Rust ones — they are what the corrections log and any later
 * rules-vs-model comparison are keyed on.
 */

import { Category } from './category.js';

/** Money and records: things you may need to find again in two years. */
const TRANSACTIONAL_KEYWORDS = [
  'rechnung',
  'quittung',
  'beleg',
  'zahlung',
  'lastschrift',
  'mahnung',
  'bestellung',
  'bestätigung',
  'bestaetigung',
  'auftrag',
  'vertrag',
  'kontoauszug',
  'abschlag',
  'steuer',
  'bescheid',
  'gutschrift',
  'zahlungseingang',
  'invoice',
  'receipt',
  'order confirmation',
  'payment',
  'statement',
  'billing',
  'refund',
  'your order',
  'subscription renewal',
];

/** Someone wants you to buy something. */
const MARKETING_KEYWORDS = [
  'rabatt',
  'angebot',
  'gutschein',
  'aktion',
  'gewinnspiel',
  'exklusiv',
  'jetzt kaufen',
  'jetzt sichern',
  'kostenlos testen',
  'black friday',
  '% off',
  '% rabatt',
  'sale',
  'discount',
  'deal',
  'save big',
  'limited time',
  'free shipping',
  'shop now',
  'last chance',
  'unbeatable',
];

/** Local parts that mean "a machine sent this and nobody reads replies". */
const AUTOMATED_LOCAL_PARTS = [
  'noreply',
  'no-reply',
  'no_reply',
  'donotreply',
  'do-not-reply',
  'mailer-daemon',
  'postmaster',
  'bounce',
  'bounces',
  'notification',
  'notifications',
  'alert',
  'alerts',
  'automated',
  'cron',
  'jenkins',
  'ci',
  'cpanel',
  'nextcloud',
  'wordpress',
  'jira',
  'confluence',
];

/**
 * The same, written as one word inside a longer address:
 * `cloudplatform-noreply`, `nichtantworten.jamobil`, `keine-antwort`.
 */
const AUTOMATED_WORDS = [
  'noreply',
  'donotreply',
  'nichtantworten',
  'keineantwort',
  'noanswer',
  'mailerdaemon',
];

/**
 * Local parts of an address that belong to a role or a department: nobody
 * in particular writes from `info@` or `kundenbetreuung@`.
 */
const ROLE_LOCAL_PARTS = [
  'info',
  'service',
  'services',
  'kundenservice',
  'kundenbetreuung',
  'kunden',
  'kontakt',
  'contact',
  'hello',
  'hallo',
  'team',
  'support',
  'help',
  'hilfe',
  'news',
  'newsletter',
  'mailing',
  'marketing',
  'sales',
  'vertrieb',
  'shop',
  'store',
  'order',
  'orders',
  'bestellung',
  'bestellungen',
  'rechnung',
  'rechnungen',
  'invoice',
  'invoices',
  'billing',
  'buchhaltung',
  'account',
  'accounts',
  'konto',
  'security',
  'sicherheit',
  'update',
  'updates',
  'office',
  'online',
  'webmaster',
  'admin',
  'feedback',
  'community',
  'events',
  'presse',
  'press',
  'jobs',
  'karriere',
  'careers',
  'booking',
  'bookings',
  'reservierung',
  'reservations',
  'tickets',
  'customer',
  'customerservice',
  'care',
  'mitglieder',
  'members',
  'verwaltung',
  'zentrale',
  'empfang',
  'abo',
  'aboservice',
  'leserservice',
];

/**
 * First labels of a sending domain that exist to send bulk mail:
 * `news.traderepublic.com`, `email.feverup.com`, `send.barneysfarm.com`.
 * Not `mail.` or `nachrichten.`: platforms relay what people write through
 * those (a buyer on Kleinanzeigen, a landlord on ImmoScout).
 */
const BULK_SUBDOMAINS = [
  'news',
  'newsletter',
  'newsletters',
  'email',
  'e',
  'em',
  'send',
  'mailing',
  'mailings',
  'mailer',
  'marketing',
  'promo',
  'promotions',
  'angebote',
  'campaign',
  'campaigns',
  'mkt',
  'crm',
  'info',
  'loyalty',
  'offers',
  'deals',
  'mc',
  'sg',
];

/** Display names that are a department, not a person. */
const ROLE_WORDS = [
  'team',
  'support',
  'info',
  'service',
  'kundenservice',
  'sales',
  'marketing',
  'news',
  'billing',
  'buchhaltung',
  'admin',
  'office',
  'kontakt',
  'hilfe',
  'help',
  'noreply',
  'no-reply',
  'shop',
  'store',
];

/** Reply prefixes, including the German "AW:" and the Scandinavian "SV:". */
const REPLY_PREFIXES = ['re:', 'aw:', 'antw:', 'sv:', 'fwd:', 'wg:'];

/**
 * Scores a message's signals.
 *
 * @param {import('./facts.js').MessageFacts} facts
 * @returns {Array<{rule: string, detail: string, category: string, weight: number}>}
 */
export function evaluate(facts) {
  const reasons = [];

  const subject = (facts.subject ?? '').toLowerCase();
  const snippet = (facts.snippet ?? '').toLowerCase();
  const precedenceIsBulk = (() => {
    if (facts.precedence == null) return false;
    const p = facts.precedence.trim().toLowerCase();
    return p === 'bulk' || p === 'list' || p === 'junk';
  })();

  // Whether this arrived as bulk mail at all. Personal heuristics are
  // suppressed when it did: newsletters are routinely sent from a real
  // person's name to one recipient, and would otherwise read as personal.
  const bulkDomain = facts.fromAddr != null ? bulkSubdomain(facts.fromAddr) : null;
  const bulk =
    facts.listId != null ||
    precedenceIsBulk ||
    facts.listUnsubscribe != null ||
    bulkDomain != null;

  // -- bulk shape ---------------------------------------------------------

  if (facts.listId != null) {
    reasons.push({
      rule: 'header.list_id',
      detail: `sent through the mailing list ${facts.listId}`,
      category: Category.Newsletter,
      weight: 2.5,
    });
  }

  if (precedenceIsBulk) {
    reasons.push({
      rule: 'header.precedence',
      detail: 'marked Precedence: bulk',
      category: Category.Newsletter,
      weight: 1.0,
    });
  }

  if (facts.listUnsubscribe != null) {
    // A List-Unsubscribe without a List-Id is the shape of a marketing
    // blast: bulk enough to need an opt-out, not a real list.
    const [category, weight, detail] =
      facts.listId != null
        ? [Category.Newsletter, 1.0, 'offers a List-Unsubscribe link']
        : [
            Category.Marketing,
            2.0,
            'offers a List-Unsubscribe link but is not a real mailing list',
          ];
    reasons.push({ rule: 'header.list_unsubscribe', detail, category, weight });
  }

  if (bulkDomain != null && facts.listId == null && facts.listUnsubscribe == null) {
    // Weaker than a List-Unsubscribe: the name of a domain is a habit, not a
    // header that exists for the purpose.
    reasons.push({
      rule: 'sender.bulk_domain',
      detail: `sent from ${bulkDomain}, a domain for bulk mail`,
      category: Category.Marketing,
      weight: 1.5,
    });
  }

  // -- automation ---------------------------------------------------------

  if (facts.autoSubmitted != null) {
    if (facts.autoSubmitted.trim().toLowerCase() !== 'no') {
      reasons.push({
        rule: 'header.auto_submitted',
        detail: `declares Auto-Submitted: ${facts.autoSubmitted.trim()}`,
        category: Category.Notification,
        weight: 3.0,
      });
    }
  }

  if (facts.fromAddr != null) {
    const local = facts.fromAddr.split('@')[0].toLowerCase();
    if (isAutomatedAddress(local)) {
      reasons.push({
        rule: 'sender.automated',
        detail: `sent from the unattended address ${local}@…`,
        category: Category.Notification,
        weight: 2.5,
      });
    }
  }

  // -- subject and body keywords -----------------------------------------

  const transactionalSubject = firstMatch(subject, TRANSACTIONAL_KEYWORDS);
  if (transactionalSubject != null) {
    reasons.push({
      rule: 'subject.transactional',
      detail: `subject mentions “${transactionalSubject}”`,
      category: Category.Transactional,
      // Above a List-Id and its List-Unsubscribe together: a bill sent
      // through a utility's mailing system is still the bill.
      weight: 4.0,
    });

    // A document attached to something that reads like a bill is usually
    // the bill.
    if (facts.hasAttachments) {
      reasons.push({
        rule: 'attachment.with_transactional_subject',
        detail: 'carries an attachment',
        category: Category.Transactional,
        weight: 1.0,
      });
    }
  } else {
    const transactionalBody = firstMatch(snippet, TRANSACTIONAL_KEYWORDS);
    if (transactionalBody != null) {
      reasons.push({
        rule: 'body.transactional',
        detail: `body mentions “${transactionalBody}”`,
        category: Category.Transactional,
        weight: 1.0,
      });
    }
  }

  const marketingSubject = firstMatch(subject, MARKETING_KEYWORDS);
  if (marketingSubject != null) {
    reasons.push({
      rule: 'subject.marketing',
      detail: `subject mentions “${marketingSubject}”`,
      category: Category.Marketing,
      weight: 3.5,
    });
  } else {
    const marketingBody = firstMatch(snippet, MARKETING_KEYWORDS);
    if (marketingBody != null) {
      reasons.push({
        rule: 'body.marketing',
        detail: `body mentions “${marketingBody}”`,
        category: Category.Marketing,
        weight: 1.0,
      });
    }
  }

  // -- personal shape -----------------------------------------------------

  // A reply is strong evidence even on a list: someone typed it to you.
  if (facts.inReplyTo != null) {
    reasons.push({
      rule: 'thread.reply',
      detail: 'is a reply to an existing thread',
      category: Category.Personal,
      weight: 2.0,
    });
  }

  if (!bulk) {
    if (REPLY_PREFIXES.some((prefix) => subject.startsWith(prefix))) {
      reasons.push({
        rule: 'subject.reply_prefix',
        detail: 'subject is a reply',
        category: Category.Personal,
        weight: 1.5,
      });
    }

    const organisation =
      (facts.fromAddr != null &&
        (isRoleAddress(facts.fromAddr) || isCompanyAddress(facts.fromAddr))) ||
      (facts.fromName != null && namesTheDomain(facts.fromName, facts.fromAddr));
    if (facts.fromName != null && !organisation) {
      if (looksLikeAPerson(facts.fromName, facts.fromAddr)) {
        reasons.push({
          rule: 'sender.person',
          detail: `${facts.fromName} looks like a person, not a department`,
          category: Category.Personal,
          weight: 1.5,
        });
      }
    }

    if (facts.recipientCount === 1 && !organisation) {
      reasons.push({
        rule: 'recipients.single',
        detail: 'addressed only to you',
        category: Category.Personal,
        weight: 0.75,
      });
    }
  }

  return reasons;
}

/** The bulk-sending domain a sender writes from, if any. */
function bulkSubdomain(addr) {
  const at = addr.lastIndexOf('@');
  if (at < 0) return null;
  const domain = addr.slice(at + 1).trim().toLowerCase();
  const labels = domain.split('.');
  // `news.example.com`, not `example.com`: the label must be a subdomain.
  return labels.length >= 3 && BULK_SUBDOMAINS.includes(labels[0]) ? domain : null;
}

/** The words of an address's local part, without trailing numbers. */
function segments(local) {
  return local
    .split(/[.\-_+]/)
    .map((segment) => segment.replace(/\d+$/, ''))
    .filter((segment) => segment !== '');
}

/** An address that is a role, not a person: `info@`, `service-center@`. */
function isRoleAddress(addr) {
  const local = addr.split('@')[0].toLowerCase();
  return segments(local).some((segment) => ROLE_LOCAL_PARTS.includes(segment));
}

/** uber@uber.com: an address that is the company's own name. */
function isCompanyAddress(addr) {
  const at = addr.lastIndexOf('@');
  if (at < 0) return false;
  const local = addr.slice(0, at).toLowerCase();
  const labels = addr.slice(at + 1).toLowerCase().split('.');
  return labels.slice(0, -1).includes(local);
}

/** An address kept by a machine: `noreply@`, `calendar-notification@`. */
function isAutomatedAddress(local) {
  const compact = local.replace(/[^a-z0-9]/g, '');
  return (
    segments(local).some((segment) => AUTOMATED_LOCAL_PARTS.includes(segment)) ||
    AUTOMATED_WORDS.some((word) => compact.includes(word))
  );
}

/**
 * A display name that is the sending domain's name is the organisation
 * speaking — unless the address is the person's own (Anna Weber at
 * anna@weber.de), or the name says a platform relayed it.
 */
function namesTheDomain(name, addr) {
  if (addr == null) return false;
  const at = addr.lastIndexOf('@');
  if (at < 0) return false;
  const lowered = name.toLowerCase();
  if ([' über ', ' via ', '(via '].some((relay) => lowered.includes(relay))) return false;
  const labels = addr.slice(at + 1).toLowerCase().split('.');
  const body = labels.slice(0, -1).join('').replace(/-/g, '');
  const local = addr.slice(0, at).toLowerCase();
  const tokens = lowered.split(/[^\p{L}\p{N}]+/u).filter((t) => [...t].length >= 4);
  return tokens.some((t) => body.includes(t)) && !tokens.some((t) => local.includes(t));
}

function firstMatch(haystack, needles) {
  return needles.find((n) => haystack.includes(n)) ?? null;
}

/**
 * A display name with a given and family name, that is not a department and
 * not just the address repeated back.
 */
function looksLikeAPerson(name, addr) {
  const trimmed = name.trim();
  if (trimmed === '' || trimmed.includes('@')) return false;

  // Many senders put the address in the display name; that says nothing.
  if (addr != null && trimmed.toLowerCase() === addr.toLowerCase()) return false;

  const words = trimmed.split(/\s+/);
  const lowered = words.map((word) => word.toLowerCase());
  if (ROLE_WORDS.some((role) => lowered.includes(role))) return false;

  // "Anna Weber" yes; "Rust Weekly" also matches this shape, which is why
  // this only applies when nothing marked the message as bulk.
  return words.length >= 2;
}
