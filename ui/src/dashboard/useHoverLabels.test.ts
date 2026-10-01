import test from 'node:test';
import assert from 'node:assert/strict';
import { hoverRevision } from './useHoverLabels.ts';
import type { LookRead } from './looks';

test('a tooltip revision changes with the question and with every answer, whatever the sizes', () => {
  const answers = new Map<string, LookRead>([
    ['A', { style: { name: 'Alpha', color: null }, flag: 'data:image/png;base64,AAAA' }],
    ['B', { style: { name: 'Beta', color: null }, flag: 'data:image/png;base64,BBBB' }],
  ]);
  const read = (tag: string): LookRead => answers.get(tag) ?? { style: undefined, flag: undefined };
  // Both warmed at one hour: hovering B then A changes the revision, though every map is one entry.
  const b = hoverRevision({ hour: 100, tags: ['B'] }, read);
  const a = hoverRevision({ hour: 100, tags: ['A'] }, read);
  assert.notEqual(a, b);
  // The same question at another hour is another revision.
  assert.notEqual(hoverRevision({ hour: 101, tags: ['A'] }, read), a);
  // Each part on its own: still going, finished with nothing, then answered.
  const at = (look: LookRead) => hoverRevision({ hour: 100, tags: ['C'] }, () => look);
  const revisions = [
    at({ style: undefined, flag: undefined }),
    at({ style: null, flag: undefined }),
    at({ style: undefined, flag: null }),
    at({ style: null, flag: null }),
    at({ style: { name: 'Gamma', color: null }, flag: undefined }),
    at({ style: { name: 'Gamma', color: null }, flag: null }),
    at({ style: { name: 'Gamma', color: null }, flag: 'data:x' }),
    at({ style: { name: 'Gamma', color: [1, 2, 3] }, flag: 'data:x' }),
    at({ style: undefined, flag: 'data:x' }),
  ];
  assert.equal(new Set(revisions).size, revisions.length);
  // Nothing asked: an empty revision, and the same answers give the same revision.
  assert.equal(hoverRevision(null, read), '');
  assert.equal(hoverRevision({ hour: 100, tags: ['A'] }, read), a);
});
