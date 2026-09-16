/**
 * Reading a Thunderbird `MessagePart` tree.
 *
 * The API hands back a nested part tree rather than the flat headers the Rust
 * parser produced, so this is new code and it is where the body signals live
 * or die. HTML-only mail matters most: it is a large share of marketing, and
 * skipping it would disable the body keywords on exactly the mail they exist
 * for — silently, and in a way no test of the classifier would catch.
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { bodyText, hasAttachments } from '../hosts/thunderbird/messages.js';

const plain = {
  contentType: 'message/rfc822',
  headers: { subject: ['Termin'] },
  parts: [{ contentType: 'text/plain', body: 'Dienstag passt.' }],
};

const multipart = {
  contentType: 'multipart/alternative',
  parts: [
    { contentType: 'text/html', body: '<p>Dienstag <b>passt</b>.</p>' },
    { contentType: 'text/plain', body: 'Dienstag passt.' },
  ],
};

const htmlOnly = {
  contentType: 'text/html',
  body: "<html><body><h1>20% off</h1><p>Nur für kurze Zeit: <b>20%</b> Rabatt.</p><a href='#'>Jetzt shoppen</a></body></html>",
};

test('plain text is read straight off the part', () => {
  assert.equal(bodyText(plain), 'Dienstag passt.');
});

test('the plain part wins over the html one, whatever the order', () => {
  assert.equal(bodyText(multipart), 'Dienstag passt.');
});

test('html-only mail is flattened rather than skipped', () => {
  const text = bodyText(htmlOnly);
  assert.ok(text.includes('Rabatt'), `keywords should survive: ${text}`);
  assert.ok(!text.includes('<'), 'no markup should be left');
});

test('script and style content does not become body text', () => {
  // Otherwise a tracking script's contents get scored as though someone wrote
  // them, and a stylesheet full of `sale` class names reads as marketing.
  const text = bodyText({
    contentType: 'text/html',
    body: '<style>.sale{color:red}</style><script>var deal=1;</script><p>Hallo</p>',
  });
  assert.equal(text.trim(), 'Hallo');
});

test('entities are decoded', () => {
  assert.equal(
    bodyText({ contentType: 'text/html', body: '<p>Preis &lt; 10&nbsp;&amp; frei</p>' }).trim(),
    'Preis < 10 & frei',
  );
});

test('a message with no body at all is null, not an empty string', () => {
  assert.equal(bodyText({ contentType: 'multipart/mixed', parts: [] }), null);
  assert.equal(bodyText(null), null);
});

test('an attachment is found by name', () => {
  assert.ok(
    hasAttachments({
      contentType: 'multipart/mixed',
      parts: [
        { contentType: 'text/plain', body: 'anbei' },
        { contentType: 'application/pdf', name: 'rechnung.pdf' },
      ],
    }),
  );
});

test('an attachment with no name is found by its disposition', () => {
  assert.ok(
    hasAttachments({
      contentType: 'multipart/mixed',
      parts: [
        { contentType: 'text/plain', body: 'anbei' },
        {
          contentType: 'application/octet-stream',
          headers: { 'content-disposition': ['attachment'] },
        },
      ],
    }),
  );
});

test('an inline image is not an attachment', () => {
  // Marketing mail is full of inline images and tracking pixels. Counting them
  // would fire the attachment signal on nearly every bulk message.
  assert.equal(
    hasAttachments({
      contentType: 'multipart/related',
      parts: [
        { contentType: 'text/html', body: '<img src="cid:x">' },
        {
          contentType: 'image/gif',
          // Thunderbird sets `name` on inline parts as well, so the name alone
          // cannot be what decides this.
          name: 'pixel.gif',
          headers: { 'content-disposition': ['inline; filename="pixel.gif"'] },
        },
      ],
    }),
    false,
  );
});

test('the message body is never itself an attachment', () => {
  // A text part often carries a name, and Thunderbird names the root part too.
  // Counting either would fire the attachment signal on plain mail.
  assert.equal(
    hasAttachments({
      contentType: 'multipart/alternative',
      name: 'message.eml',
      parts: [
        { contentType: 'text/plain', name: 'body.txt', body: 'Hallo' },
        { contentType: 'text/html', name: 'body.html', body: '<p>Hallo</p>' },
      ],
    }),
    false,
  );
});
