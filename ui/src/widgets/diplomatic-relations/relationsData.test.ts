import assert from 'node:assert/strict';
import test from 'node:test';
import type { CountryGroupReply } from '../../generated/dashboard';
import { opinionDomain, opinionIndex, opinionText, orderCountries, pairHolders, pairLine, pairOf, pairToken, topTags } from './relationsData.ts';
import type { ScoreboardPreparedReply, ScoreboardSeries } from '../../generated/dashboard';
import type { DiplomacyOpinionsReply } from '../../generated/dashboard';

const reply = { hour: 10, unresolved: 0, warnings: [], owners: [
  { tag: 'A', readHour: 8, known: true, unknownTargets: [], opinions: [{ target: 'B', value: 40 }, { target: 'A', value: 5 }] },
  { tag: 'B', readHour: null, known: false, unknownTargets: [], opinions: [] },
  { tag: 'C', readHour: 10, known: true, unknownTargets: ['D'], opinions: [{ target: 'A', value: -120 }] },
  { tag: 'D', readHour: null, known: false, unknownTargets: [], opinions: [] },
] } as unknown as DiplomacyOpinionsReply;

test('a known owner’s unlisted target is exact zero, an unknown owner’s row is unknown, and self-pairs are kept', () => {
  const at = opinionIndex(reply);
  assert.deepEqual(at('A', 'B'), { value: 40, readHour: 8 });
  assert.deepEqual(at('A', 'C'), { value: 0, readHour: 8 }, 'missing from a complete owner read is zero');
  assert.deepEqual(at('B', 'A'), { unknown: true }, 'an owner without proof is unknown, never zero');
  assert.deepEqual(at('A', 'A'), { value: 5, readHour: 8 }, 'the diagonal is recorded state');
  assert.equal(at('Z', 'A'), null);
  assert.equal(at('A', 'Z'), null, 'a country the reply was not asked about is not zero');
  assert.deepEqual(opinionDomain(reply), [-120, 40], 'the colour domain is the recorded extremes, not a fixed range');
  assert.equal(opinionText(40), '+40'); assert.equal(opinionText(-3), '-3'); assert.equal(opinionText(0), '0');
});

test('a target the owner’s read cannot prove is unknown while the owner’s other pairs stay known', () => {
  const at = opinionIndex(reply);
  assert.deepEqual(at('C', 'D'), { unknown: true });
  assert.deepEqual(at('C', 'A'), { value: -120, readHour: 10 });
  assert.deepEqual(at('C', 'B'), { value: 0, readHour: 10 });
  assert.deepEqual(at('D', 'C'), { unknown: true });
});

test('rows order by faction then name, countries outside any faction last; or by name', () => {
  const groups = { factions: [
    { id: '2', members: [{ tag: 'C', firstHour: 0, lastHour: 20 }, { tag: 'A', firstHour: 0, lastHour: 20 }] },
    { id: '1', members: [{ tag: 'D', firstHour: 0, lastHour: 20 }] },
  ] } as unknown as CountryGroupReply;
  const names: Record<string, string> = { A: 'Zeta', B: 'Alpha', C: 'Beta', D: 'Gamma' };
  const factions: Record<string, string> = { '1': 'A mod faction', '2': 'Second' };
  assert.deepEqual(orderCountries(['A', 'B', 'C', 'D'], 'faction', tag => names[tag], groups, 10, id => factions[id]), ['D', 'C', 'A', 'B']);
  assert.deepEqual(orderCountries(['A', 'B', 'C', 'D'], 'name', tag => names[tag], groups, 10, id => factions[id]), ['B', 'C', 'D', 'A']);
  assert.deepEqual(orderCountries(['A', 'C'], 'faction', tag => names[tag], groups, 30, id => factions[id]), ['C', 'A'], 'after the rosters end, by name');
});

test('pair lines keep known values, break at unknown stretches, and a saved pair round-trips', () => {
  const line = pairLine([{ firstHour: 0, lastHour: 9, value: 10, startsSegment: false }, { firstHour: 10, lastHour: 19, value: null, startsSegment: false },
    { firstHour: 20, lastHour: 29, value: -5, startsSegment: false }, { firstHour: 30, lastHour: 39, value: 0, startsSegment: false }]);
  assert.deepEqual(line.map(run => [run.firstHour, run.value, run.breakBefore ?? false]), [[0, 10, true], [20, -5, true], [30, 0, false]]);
  assert.deepEqual(pairOf(pairToken({ first: 'A', second: 'mod:Ω', hour: 120 })), { first: 'A', second: 'mod:Ω', hour: 120 }, 'the pin keeps the hour that names its holders');
  assert.equal(pairOf(JSON.stringify(['A', 'B'])), null, 'a pair without its hour cannot name its holders');
  assert.equal(pairOf('not json'), null); assert.equal(pairOf(null), null);
  assert.deepEqual(pairHolders({ first: 'A', second: 'A', hour: 48 }).map(holder => [holder.key, holder.hour]), [['first', 48], ['second', 48]],
    'both holders are read at the chosen hour itself, never a sample before a reuse of the tag');
});

test('a top N ranks only once its hour and measure are prepared', () => {
  const factories = { key: 'factories', label: 'Factories', unit: 'factories', decimals: 0, available: true, reason: null } as const;
  const entry = (tag: string, slot: number, amount: string): ScoreboardSeries => ({ tag, segments: [{ firstHour: 10, lastHour: 10 }],
    identities: [{ firstHour: 10, lastHour: 10, slot, countryId: slot + 10 }], membership: null,
    columns: [{ key: 'factories', runs: [{ firstHour: 10, lastHour: 10, amount, startsSegment: false }], readRuns: [{ firstHour: 10, lastHour: 10, readHour: 10, hourly: true }] }] });
  const reply = (series: ScoreboardSeries[], prefix = {}): ScoreboardPreparedReply => ({ hour: 10, maxima: { factories: '9' }, ...prefix,
    data: { branchId: '1', defaultColumns: ['factories'], defaultSort: 'factories', columns: [factories], series, warnings: [] } }) as ScoreboardPreparedReply;
  const pending = { coverage: { firstHour: 10, lastHour: 9, complete: false }, pending: [{ firstHour: 10, lastHour: 10 }] };
  assert.equal(topTags(reply([], pending), 10, 'factories'), null, 'an empty pending index is not an empty ranking');
  assert.equal(topTags(reply([entry('A', 1, '3')], { preparing: ['factories'] }), 10, 'factories'), null);
  assert.deepEqual(topTags(reply([entry('A', 1, '3'), entry('B', 2, '9')]), 10, 'factories'), ['B', 'A'], 'the prepared index ranks highest first');
});
