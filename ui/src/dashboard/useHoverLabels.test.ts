import test from 'node:test';
import assert from 'node:assert/strict';
import { heldRead, hoverRevision } from './useHoverLabels.ts';
import { FAILED_FACT } from './countryFacts.ts';
import type { HoverRead } from './useHoverLabels';

test('a tooltip revision changes with the question and with every answer, whatever the sizes', () => {
  const answers = new Map<string, HoverRead>([
    ['A', { style: { name: 'Alpha', color: null }, flag: { url: 'data:image/png;base64,AAAA' } }],
    ['B', { style: { name: 'Beta', color: null }, flag: { url: 'data:image/png;base64,BBBB' } }],
  ]);
  const read = (tag: string): HoverRead => answers.get(tag) ?? { style: undefined, flag: undefined };
  // Both warmed at one hour: hovering B then A changes the revision, though every map is one entry.
  const b = hoverRevision({ hour: 100, tags: ['B'] }, read);
  const a = hoverRevision({ hour: 100, tags: ['A'] }, read);
  assert.notEqual(a, b);
  // The same question at another hour is another revision.
  assert.notEqual(hoverRevision({ hour: 101, tags: ['A'] }, read), a);
  // Each part on its own: still going, finished with nothing, then answered.
  const at = (look: HoverRead) => hoverRevision({ hour: 100, tags: ['C'] }, () => look);
  const revisions = [
    at({ style: undefined, flag: undefined }),
    at({ style: null, flag: undefined }),
    at({ style: undefined, flag: null }),
    at({ style: null, flag: null }),
    at({ style: { name: 'Gamma', color: null }, flag: undefined }),
    at({ style: { name: 'Gamma', color: null }, flag: null }),
    at({ style: { name: 'Gamma', color: null }, flag: { url: 'data:x' } }),
    at({ style: { name: 'Gamma', color: [1, 2, 3] }, flag: { url: 'data:x' } }),
    at({ style: undefined, flag: { url: 'data:x' } }),
    at({ style: undefined, flag: { reason: 'Missing native sprite' } }),
    at({ style: undefined, flag: { reason: 'Unreadable native sprite' } }),
    at({ style: FAILED_FACT, flag: undefined }),
    at({ style: undefined, flag: FAILED_FACT }),
    at({ style: FAILED_FACT, flag: null }),
    at({ style: null, flag: FAILED_FACT }),
    at({ style: FAILED_FACT, flag: FAILED_FACT }),
  ];
  assert.equal(new Set(revisions).size, revisions.length);
  // Nothing asked: an empty revision, and the same answers give the same revision.
  assert.equal(hoverRevision(null, read), '');
  assert.equal(hoverRevision({ hour: 100, tags: ['A'] }, read), a);
});

test('a hovered hour still being read shows its sampled look, part by part, never a failed or empty one', () => {
  const sample: HoverRead = { style: { name: 'Sampled', color: [1, 2, 3] }, flag: { url: 'data:sampled' } };
  const going: HoverRead = { style: undefined, flag: undefined };
  assert.deepEqual(heldRead(going, sample), sample);
  const exact: HoverRead = { style: { name: 'Exact', color: null }, flag: undefined };
  assert.deepEqual(heldRead(exact, sample), { style: exact.style, flag: sample.flag });
  assert.deepEqual(heldRead({ style: null, flag: FAILED_FACT }, sample), { style: null, flag: FAILED_FACT }, 'a completed read wins');
  assert.deepEqual(heldRead(going, { style: FAILED_FACT, flag: { reason: 'No flag file' } }), going);
  assert.deepEqual(heldRead(going, { style: null, flag: null }), going);
});
