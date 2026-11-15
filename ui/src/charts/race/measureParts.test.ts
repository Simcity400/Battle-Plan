import assert from 'node:assert/strict';
import test from 'node:test';
import { CELL, parseFollowed } from '../../dashboard/follow/message.ts';
import { message } from '../../dashboard/follow/messages.fixture.ts';
import { drawnItems, hatchEntry, hatchesOf, keyedAnswer, keyedIds, keyedStates, measureAnswer, measureParts, measureStates, splitEntry } from './measureParts.ts';

const answer = (rows: [string, bigint, bigint, number?][]) => parseFollowed(message(10, [{ id: 1, answer: { keys: rows.map(([, key]) => key), columns: 1,
  values: rows.map(([, , value]) => value), states: rows.map(([, , , state]) => state ?? CELL.value), names: rows.map(([tag]) => tag) } }])).answers[0]!.answer;
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

test('followed and asked part values are read by tag, and by part key for a sub-keyed series; a missing cell is null', () => {
  const one = measureAnswer(answer([['GER', 1n, 1200n], ['ITA', 2n, 0n, CELL.unrecorded]]), ['GER', 'ITA'], 2);
  assert.deepEqual([...one], [['GER', 12], ['ITA', null]]);
  const keyed = keyedAnswer(answer([['USA', 1n << 32n | 0n, 5n], ['USA', 1n << 32n | 1n, 2n], ['SOV', 2n << 32n | 0n, 0n, CELL.missing]]), 32, 0);
  assert.deepEqual([...keyed.get('USA')!], [['0', 5], ['1', 2]]);
  assert.deepEqual([...keyed.get('SOV')!], [['0', null]]);
  assert.deepEqual(keyedIds(keyed.values()), ['0', '1']);
  const states = { branchId: '1', measure: 'x', decimals: 0, states: [{ tag: 'GER', hour: 7, run: { valueRaw: '4', firstHour: 0, lastHour: 9 }, drop: null, missing: null },
    { tag: 'ITA', hour: 7, run: null, drop: null, missing: 'not recorded' }, { tag: 'GER', hour: 8, run: { valueRaw: '9', firstHour: 8, lastHour: 9 }, drop: null, missing: null }] };
  assert.deepEqual([...measureStates(states as never, 7)], [['GER', 4], ['ITA', null]]);
  const parts = measureParts([{ id: 'army', label: 'Army', measure: { kind: 'k', measure: 'army' } }, { id: 'navy', label: 'Navy', measure: { kind: 'k', measure: 'navy' } }],
    new Map([['army', measureStates(states as never, 7)]]), ['GER', 'ITA']);
  assert.deepEqual([...parts.get('GER')!], [['army', 4], ['navy', null]], 'a part never answered is not known, never zero');
  const keyedReply = { ...states, states: [{ ...states.states[0], part: '1' }, { ...states.states[1], part: '0' }] };
  assert.deepEqual([...keyedStates(keyedReply as never, 7)].map(([tag, held]) => [tag, [...held]]), [['GER', [['1', 4]]], ['ITA', [['0', null]]]]);
});
