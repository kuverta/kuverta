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
  const bulk =
    facts.listId != null || precedenceIsBulk || facts.listUnsubscribe != null;

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
    if (
      AUTOMATED_LOCAL_PARTS.some(
        (needle) => local === needle || local.startsWith(`${needle}-`),
      )
    ) {
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
      weight: 3.0,
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

    if (facts.fromName != null) {
      if (looksLikeAPerson(facts.fromName, facts.fromAddr)) {
        reasons.push({
          rule: 'sender.person',
          detail: `${facts.fromName} looks like a person, not a department`,
          category: Category.Personal,
          weight: 1.5,
        });
      }
    }

    if (facts.recipientCount === 1) {
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
