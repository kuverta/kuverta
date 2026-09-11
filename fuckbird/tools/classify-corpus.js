/**
 * Classifies a corpus dumped by `../docker/fill-mailbox.py --dump` and reports the
 * distribution.
 *
 * This exists to keep the JS port honest. The brief quotes what the Rust
 * classifier produced on this same seeded corpus; if this prints something
 * else, the port has drifted and the corrections collected under it stop being
 * comparable with anything measured before.
 *
 *   python3 ../docker/fill-mailbox.py --dump 250 | node tools/classify-corpus.js
 *
 * Pass `--reasons` to also see which rules fired how often, which is the
 * quickest way to find a signal that has stopped matching.
 */

import { Classifier } from '../core/classify.js';
import { buildFacts } from '../core/facts.js';
import { ALL_CATEGORIES } from '../core/category.js';

const showReasons = process.argv.includes('--reasons');

const input = await readStdin();
const rows = input
  .split('\n')
  .filter((line) => line.trim() !== '')
  .map((line) => JSON.parse(line));

if (rows.length === 0) {
  console.error('no messages on stdin — pipe `../docker/fill-mailbox.py --dump` in');
  process.exit(1);
}

const classifier = Classifier.withoutHistory();
const counts = new Map(ALL_CATEGORIES.map((c) => [c, 0]));
const ruleCounts = new Map();
let confidenceTotal = 0;

for (const row of rows) {
  const { category, confidence, reasons } = classifier.classify(buildFacts(row));
  counts.set(category, counts.get(category) + 1);
  confidenceTotal += confidence;
  for (const reason of reasons) {
    ruleCounts.set(reason.rule, (ruleCounts.get(reason.rule) ?? 0) + 1);
  }
}

console.log(`${rows.length} messages\n`);
for (const [category, count] of [...counts].sort((a, b) => b[1] - a[1])) {
  if (count === 0) continue;
  const share = ((count / rows.length) * 100).toFixed(1).padStart(5);
  console.log(`  ${category.padEnd(14)} ${String(count).padStart(4)}  ${share}%`);
}
console.log(`\n  mean confidence ${(confidenceTotal / rows.length).toFixed(3)}`);

if (showReasons) {
  console.log('\nrules that carried a verdict:');
  for (const [rule, count] of [...ruleCounts].sort((a, b) => b[1] - a[1])) {
    console.log(`  ${rule.padEnd(38)} ${String(count).padStart(4)}`);
  }
}

function readStdin() {
  return new Promise((resolve, reject) => {
    let data = '';
    process.stdin.setEncoding('utf8');
    process.stdin.on('data', (chunk) => (data += chunk));
    process.stdin.on('end', () => resolve(data));
    process.stdin.on('error', reject);
  });
}
