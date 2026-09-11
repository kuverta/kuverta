/**
 * The Thunderbird adapter against the host contract.
 *
 * The same suite the standalone client's adapter runs and the same suite the
 * in-memory reference runs. That is the whole compatibility claim: three
 * hosts, one contract, one triage surface written against it.
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { runConformance } from './support/conformance.js';
import { fakeMessenger } from './support/fake-messenger.js';
import { ThunderbirdHost } from '../hosts/thunderbird/host.js';
import { tagKeyFor } from '../hosts/thunderbird/tags.js';

runConformance('Thunderbird', () => new ThunderbirdHost(fakeMessenger()));

// -- things specific to this host ---------------------------------------

test('Thunderbird: Everything leaves out Archive and Trash', async () => {
  // Otherwise archiving a message leaves it exactly where it was, and on Gmail
  // the list shows most mail twice — §3.6.
  const messenger = fakeMessenger();
  const host = new ThunderbirdHost(messenger);

  const before = await host.page({ scope: all(), offset: 0, limit: 50 });
  await host.archive(before.rows[0].id);
  const after = await host.page({ scope: all(), offset: 0, limit: 50 });

  assert.equal(after.total, before.total - 1);
});

test('Thunderbird: a folder with no SPECIAL-USE is still found by name', async () => {
  // §3.6: Dovecot advertises SPECIAL-USE but not CREATE-SPECIAL-USE, so a
  // folder another client created carries no attribute at all.
  const messenger = fakeMessenger({
    folders: [
      { id: 'f-inbox', name: 'Inbox', path: '/INBOX', specialUse: ['inbox'] },
      { id: 'f-archive', name: 'Archiv', path: '/Archiv', specialUse: [] },
    ],
  });
  const host = new ThunderbirdHost(messenger);

  const before = await host.page({ scope: all(), offset: 0, limit: 50 });
  await host.archive(before.rows[0].id);

  const moved = [...messenger._messages.values()].filter((m) => m.folderId === 'f-archive');
  assert.equal(moved.length, 1, 'should have moved into the folder found by name');
});

test('Thunderbird: with no Archive folder, archiving says so instead of guessing', async () => {
  const messenger = fakeMessenger({
    folders: [{ id: 'f-inbox', name: 'Inbox', path: '/INBOX', specialUse: ['inbox'] }],
  });
  const host = new ThunderbirdHost(messenger);
  const { rows } = await host.page({ scope: all(), offset: 0, limit: 1 });

  await assert.rejects(() => host.archive(rows[0].id), /no Archive folder/);
});

test('Thunderbird: undo finds the moved message again by its Message-ID', async () => {
  // The id it had is gone with the move; the Message-ID is the only handle
  // that survived.
  const messenger = fakeMessenger();
  const host = new ThunderbirdHost(messenger);

  const before = await host.page({ scope: all(), offset: 0, limit: 50 });
  const subject = before.rows[0].subject;
  const idBefore = before.rows[0].id;

  await host.archive(idBefore);
  await host.undo();

  const after = await host.page({ scope: all(), offset: 0, limit: 50 });
  const back = after.rows.find((row) => row.subject === subject);
  assert.ok(back, 'the message should be back in the list');
  assert.notEqual(back.id, idBefore, 'and it should have a new id, as Thunderbird gives it');
});

test('Thunderbird: a message with no Message-ID cannot be put back, and says so', async () => {
  // §3.6: a message with no Message-ID is legal and does happen. Moving the
  // wrong message back would be far worse than refusing.
  const messenger = fakeMessenger({
    seed: [
      {
        subject: 'No Message-ID here',
        author: 'x@example.com',
        date: Date.UTC(2026, 8, 11),
        read: false,
        tags: [],
        headerMessageId: null,
        folderId: 'f-inbox',
        body: '',
      },
    ],
  });
  const host = new ThunderbirdHost(messenger);
  const { rows } = await host.page({ scope: all(), offset: 0, limit: 1 });

  await host.archive(rows[0].id);
  await assert.rejects(() => host.undo(), /no Message-ID/);
});

test('Thunderbird: filing a category leaves the user own tags alone', async () => {
  const messenger = fakeMessenger();
  const host = new ThunderbirdHost(messenger);
  const { rows } = await host.page({ scope: all(), offset: 0, limit: 1 });

  await messenger.messages.update(Number(rows[0].id), { tags: ['important'] });
  await host.setCategory(rows[0].id, 'marketing');

  const header = await messenger.messages.get(Number(rows[0].id));
  assert.ok(header.tags.includes('important'), 'the user tag was dropped');
  assert.ok(header.tags.includes(tagKeyFor('marketing')));
});

function all() {
  return { kind: 'all', label: 'Everything' };
}
