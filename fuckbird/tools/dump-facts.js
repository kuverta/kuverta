/**
 * Turns a `fill-mailbox.py --dump` corpus into the facts the classifier reads.
 *
 * Separated from classification so the facts can be fed to another
 * implementation — the Rust `core-rules` this was ported from — and the two
 * verdict streams diffed message for message. Comparing built facts rather
 * than raw messages keeps the comparison about the classifier.
 */

import { buildFacts } from '../core/facts.js';

let data = '';
process.stdin.setEncoding('utf8');
for await (const chunk of process.stdin) data += chunk;

for (const line of data.split('\n')) {
  if (line.trim() === '') continue;
  const row = JSON.parse(line);
  // The label rides along untouched: it is the answer an evaluation scores
  // against, and it must never be one of the facts a classifier is shown.
  console.log(JSON.stringify({ ...buildFacts(row), label: row.label ?? null }));
}
