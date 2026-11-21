import test from 'node:test';
import assert from 'node:assert/strict';
import { barContent, barNameFits, END_MARK_ROOM } from './barLabel.ts';

test('a bar shows what its lane asks for, and its name only in place of missing art', () => {
  assert.deepEqual(barContent(true, true, true), { icon: true, name: true });
  assert.deepEqual(barContent(false, true, true), { icon: false, name: true });
  assert.deepEqual(barContent(true, false, true), { icon: true, name: false });
  assert.deepEqual(barContent(true, false, false), { icon: false, name: true });
  assert.deepEqual(barContent(false, false, true), { icon: false, name: false });
  assert.deepEqual(barContent(false, false, false), { icon: false, name: false });
});

test('a bar name leaves room for the end mark the bar draws, so a completion tick never covers it', () => {
  assert.equal(barNameFits(30, 7, 50, false), true);
  assert.equal(barNameFits(30, 7, 50, true), false);
  assert.equal(barNameFits(30, 7, 50 + END_MARK_ROOM, true), true);
  assert.equal(barNameFits(30, 7, 43, false), false, 'the plain fit keeps its 6 px end padding');
});
