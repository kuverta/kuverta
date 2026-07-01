/**
 * Tag writing.
 *
 * `messages.update` replaces a message's whole tag list, so every write here
 * is a chance to delete tagging the user did by hand. That is the §3.5 class
 * of mistake — quiet, and only noticed much later — which is why it is tested
 * against a fake rather than trusted to read correctly.
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  TAG_PREFIX,
  applyCategory,
  categoryForTagKey,
  currentCategory,
  ensureTags,
  tagKeyFor,
} from '../hosts/thunderbird/tags.js';
import { ALL_CATEGORIES, Category } from '../core/category.js';

/** Just enough `messenger` to exercise the tag paths. */
function fakeMessenger({ tags = [], messageTags = [] } = {}) {
  const created = [];
  const updates = [];
  return {
    created,
    updates,
    messages: {
      tags: {
        list: async () => tags,
        create: async (key, name, colour) => {
          created.push({ key, name, colour });
          tags.push({ key, tag: name, color: colour });
        },
      },
      get: async () => ({ id: 1, tags: messageTags }),
      update: async (id, changes) => {
        updates.push({ id, changes });
        messageTags.splice(0, messageTags.length, ...changes.tags);
      },
    },
  };
}

test('a tag key round-trips to its category', () => {
  for (const category of ALL_CATEGORIES) {
    assert.equal(categoryForTagKey(tagKeyFor(category)), category);
  }
});

test('a tag that is not ours is not claimed', () => {
  assert.equal(categoryForTagKey('important'), null);
  // Prefixed but not a category we know: still not ours to rewrite.
  assert.equal(categoryForTagKey(`${TAG_PREFIX}invoices`), null);
});

test('the six tags are created once', async () => {
  const messenger = fakeMessenger();
  await ensureTags(messenger);
  assert.equal(messenger.created.length, 6);

  // Runs on every startup, so a second run must do nothing.
  await ensureTags(messenger);
  assert.equal(messenger.created.length, 6);
});

test('a tag the user renamed is left alone', async () => {
  // Renaming our tag is allowed; recreating it would either fail or produce a
  // duplicate, and neither is what the user asked for.
  const tags = [{ key: tagKeyFor(Category.Personal), tag: 'People', color: '#123456' }];
  const messenger = fakeMessenger({ tags });
  await ensureTags(messenger);
  assert.equal(messenger.created.length, 5);
  assert.ok(!messenger.created.some((t) => t.key === tagKeyFor(Category.Personal)));
});

test('filing a message keeps the tags the user put there', async () => {
  const messageTags = ['important', 'work'];
  const messenger = fakeMessenger({ messageTags });

  await applyCategory(messenger, 1, Category.Newsletter);

  assert.deepEqual(
    [...messageTags].sort(),
    ['important', tagKeyFor(Category.Newsletter), 'work'].sort(),
  );
});

test('refiling replaces our tag rather than adding a second one', async () => {
  const messageTags = ['important', tagKeyFor(Category.Newsletter)];
  const messenger = fakeMessenger({ messageTags });

  await applyCategory(messenger, 1, Category.Marketing);

  assert.deepEqual(
    [...messageTags].sort(),
    ['important', tagKeyFor(Category.Marketing)].sort(),
  );
});

test('filing a message that is already filed writes nothing', async () => {
  // A backfill re-run touches every message in a folder. Writing anyway would
  // change every one of them for no reason.
  const messageTags = [tagKeyFor(Category.Personal)];
  const messenger = fakeMessenger({ messageTags });

  const changed = await applyCategory(messenger, 1, Category.Personal);

  assert.equal(changed, false);
  assert.equal(messenger.updates.length, 0);
});

test('the filed category is read back off a message', () => {
  assert.equal(
    currentCategory({ tags: ['work', tagKeyFor(Category.Transactional)] }),
    Category.Transactional,
  );
  assert.equal(currentCategory({ tags: ['work'] }), null);
  assert.equal(currentCategory({}), null);
});
