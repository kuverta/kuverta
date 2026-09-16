/**
 * That the pieces actually point at each other.
 *
 * None of this would need testing if the add-on could be run here, because the
 * first load would say so. It cannot be, so a broken import or a manifest
 * pointing at a file that moved would otherwise be found by installing it —
 * and this whole layout is one directory move away from that at all times.
 *
 * It is also what keeps the two hosts honest about the core: an import
 * reaching sideways from one host into the other would resolve on disk but
 * means the shared boundary has been breached, so that is checked too.
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync, readdirSync, statSync, existsSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');

/** Every source file that ships or is imported by one that does. */
function sources(dir = ROOT, found = []) {
  for (const entry of readdirSync(dir)) {
    if (['node_modules', '.git', 'test', 'tools', 'docs'].includes(entry)) continue;
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) sources(path, found);
    else if (entry.endsWith('.js')) found.push(path);
  }
  return found;
}

const IMPORT = /(?:^|\n)\s*import\s[^'"]*['"](\.[^'"]+)['"]/g;

test('every relative import resolves to a file that exists', () => {
  const broken = [];

  for (const file of sources()) {
    const text = readFileSync(file, 'utf8');
    for (const [, specifier] of text.matchAll(IMPORT)) {
      const target = resolve(dirname(file), specifier);
      if (!existsSync(target)) {
        broken.push(`${relative(ROOT, file)} -> ${specifier}`);
      }
    }
  }

  assert.deepEqual(broken, [], `imports pointing at nothing:\n${broken.join('\n')}`);
});

test('no host imports another host', () => {
  // The whole arrangement rests on hosts knowing about the core and the core
  // knowing about neither. A shortcut between two adapters would work and
  // would quietly end the compatibility claim.
  const offenders = [];

  for (const file of sources()) {
    const from = relative(ROOT, file);
    const match = from.match(/^hosts\/([^/]+)\//);
    if (!match) continue;

    const text = readFileSync(file, 'utf8');
    for (const [, specifier] of text.matchAll(IMPORT)) {
      const target = relative(ROOT, resolve(dirname(file), specifier));
      const other = target.match(/^hosts\/([^/]+)\//);
      if (other && other[1] !== match[1]) offenders.push(`${from} -> ${target}`);
    }
  }

  assert.deepEqual(offenders, []);
});

test('the core imports nothing from any host', () => {
  const offenders = [];

  for (const file of sources()) {
    const from = relative(ROOT, file);
    if (!from.startsWith('core/')) continue;

    const text = readFileSync(file, 'utf8');
    for (const [, specifier] of text.matchAll(IMPORT)) {
      const target = relative(ROOT, resolve(dirname(file), specifier));
      if (target.startsWith('hosts/')) offenders.push(`${from} -> ${target}`);
    }
  }

  assert.deepEqual(offenders, [], 'the core has learned about a host');
});

test('the core names no host anywhere, not even in a string', () => {
  // A core that branches on which host it is in has stopped being a core, and
  // that arrives as a string comparison long before it arrives as an import.
  const offenders = [];

  for (const file of sources()) {
    const from = relative(ROOT, file);
    if (!from.startsWith('core/')) continue;

    const text = readFileSync(file, 'utf8');
    // Comments are where the hosts are explained, which is allowed and useful.
    const code = text.replace(/\/\*[\s\S]*?\*\//g, '').replace(/(^|\s)\/\/.*/g, '');
    for (const name of ['messenger.', '__TAURI__', 'browser.']) {
      if (code.includes(name)) offenders.push(`${from} mentions ${name}`);
    }
  }

  assert.deepEqual(offenders, []);
});

test('every path the manifest names exists', () => {
  const manifest = JSON.parse(readFileSync(join(ROOT, 'manifest.json'), 'utf8'));
  const missing = [];

  const check = (path) => {
    if (typeof path !== 'string' || !path.endsWith('.html') && !path.endsWith('.js') && !path.endsWith('.svg')) return;
    if (!existsSync(join(ROOT, path))) missing.push(path);
  };

  const walk = (value) => {
    if (typeof value === 'string') check(value);
    else if (Array.isArray(value)) value.forEach(walk);
    else if (value && typeof value === 'object') Object.values(value).forEach(walk);
  };

  walk(manifest);
  assert.deepEqual(missing, []);
});

test('every script and stylesheet an html page loads exists', () => {
  const missing = [];
  const pages = [];

  const findPages = (dir) => {
    for (const entry of readdirSync(dir)) {
      if (['node_modules', '.git'].includes(entry)) continue;
      const path = join(dir, entry);
      if (statSync(path).isDirectory()) findPages(path);
      else if (entry.endsWith('.html')) pages.push(path);
    }
  };
  findPages(ROOT);

  // Scripts and stylesheets only. An `<a href>` is a navigation target, not a
  // resource, and one may legitimately point outside this repository — the
  // kuverta mount links back to that app's own page, which exists only once
  // the surface has been installed into it.
  const RESOURCES = [/<link\b[^>]*\bhref="([^"]+)"/g, /\bsrc="([^"]+)"/g];

  for (const page of pages) {
    const text = readFileSync(page, 'utf8');
    for (const pattern of RESOURCES) {
      for (const [, src] of text.matchAll(pattern)) {
        if (src.startsWith('http') || src.startsWith('data:')) continue;
        const target = resolve(dirname(page), src);
        if (!existsSync(target)) missing.push(`${relative(ROOT, page)} -> ${src}`);
      }
    }
  }

  assert.deepEqual(missing, [], `pages loading nothing:\n${missing.join('\n')}`);
});
