import assert from 'node:assert/strict';
import test from 'node:test';
import { drawnItems, hatchEntry, hatchesOf, splitEntry } from './measureParts.ts';

const entry = { tag: 'GER', value: 30, label: '30', stretch: null };

test('a bar splits into its parts only where every part is known, at their values or in proportion to the total', () => {
  const values = new Map([['military', 10], ['naval', 5], ['civilian', 15]]);
  assert.deepEqual(splitEntry(entry, ['military', 'naval', 'civilian'], values, false).parts, [{ id: 'military', value: 10 }, { id: 'naval', value: 5 }, { id: 'civilian', value: 15 }]);
  assert.equal(splitEntry(entry, ['military', 'naval'], new Map([['military', 10], ['naval', null]]), false).parts, undefined, 'a part not known leaves the bar whole');
  const score = splitEntry({ ...entry, value: 60 }, ['army', 'navy'], new Map([['army', 3], ['navy', 1]]), true);
  assert.deepEqual(score.parts, [{ id: 'army', value: 45 }, { id: 'navy', value: 15 }], 'a multiplied total is split by its parts\' shares');
  assert.equal(splitEntry({ ...entry, value: 60 }, ['army'], new Map([['army', 0]]), true).parts, undefined, 'no parts to share a total is drawn whole');
  assert.equal(splitEntry({ ...entry, value: -1 }, ['army'], new Map([['army', 2]]), true).parts, undefined, 'a signed total has no share geometry');
});

test('damaged factories are hatched inside their group’s bar, or the whole bar’s, only where every damaged count is known', () => {
  const items = [{ id: 'military', label: 'M', measure: { kind: 'k', measure: 'built1' } }, { id: 'civilian', label: 'C', measure: { kind: 'k', measure: 'built4' } },
    { id: 'damaged:military', label: 'D', measure: { kind: 'k', measure: 'damaged1' }, hatchOf: 'military' },
    { id: 'damaged:civilian', label: 'D', measure: { kind: 'k', measure: 'damaged4' }, hatchOf: 'civilian' }];
  assert.deepEqual(drawnItems(items).map(item => item.id), ['military', 'civilian'], 'a hatch part is no bar or layer of its own');
  const hatches = hatchesOf(items);
  const values = new Map([['military', 10], ['civilian', 20], ['damaged:military', 2], ['damaged:civilian', 0]]);
  assert.deepEqual(splitEntry(entry, ['military', 'civilian'], values, false, hatches).parts,
    [{ id: 'military', value: 10, hatched: 2, hatchInside: true }, { id: 'civilian', value: 20, hatched: 0, hatchInside: true }]);
  assert.equal(hatchEntry(entry, hatches, values).hatched, 2, 'unsplit, the bar hatches every group’s damaged count');
  assert.equal(hatchEntry(entry, hatches, new Map([['damaged:military', 2], ['damaged:civilian', null]])).hatched, undefined, 'a damaged count not known hatches nothing, never less');
  const unknownPart = splitEntry(entry, ['military', 'civilian'], new Map([['military', 10], ['civilian', null], ['damaged:military', 2], ['damaged:civilian', 1]]), false, hatches);
  assert.deepEqual([unknownPart.parts, unknownPart.hatched], [undefined, 3], 'a part not known leaves the bar whole, still hatched');
});

