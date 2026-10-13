import test from 'node:test';
import assert from 'node:assert/strict';
import { barContent, barNameFits, END_MARK_ROOM } from './barLabel.ts';

test('a bar shows what its lane asks for, and its name only in place of missing art', () => {
  // Portrait and name; name only.
  assert.deepEqual(barContent(true, true, true), { icon: true, name: true });
  assert.deepEqual(barContent(false, true, true), { icon: false, name: true });
  // Portrait only: the portrait, or the name when the content has no portrait.
  assert.deepEqual(barContent(true, false, true), { icon: true, name: false });
  assert.deepEqual(barContent(true, false, false), { icon: false, name: true });
  // Neither asked for (national spirits history): the bar stays plain, art or not.
  assert.deepEqual(barContent(false, false, true), { icon: false, name: false });
  assert.deepEqual(barContent(false, false, false), { icon: false, name: false });
});

test('a bar name leaves room for the end mark the bar draws, so a completion tick never covers it', () => {
  // A 30 px label after a 7 px inset fits a 50 px bar on its own...
  assert.equal(barNameFits(30, 7, 50, false), true);
  // ...but not once the bar also draws its tick at the end.
  assert.equal(barNameFits(30, 7, 50, true), false);
  assert.equal(barNameFits(30, 7, 50 + END_MARK_ROOM, true), true);
  assert.equal(barNameFits(30, 7, 43, false), false, 'the plain fit keeps its 6 px end padding');
});
