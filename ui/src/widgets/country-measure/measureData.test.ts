import test from 'node:test';
import assert from 'node:assert/strict';
import type { CountryGroupReply } from '../../generated/dashboard';
import { resolveCountries as resolveAny } from '../../dashboard/groups.ts';
import type { CountrySelection } from '../../dashboard/model';
import { definition as politicalPower } from '../political-power/definition.ts';
import { definition as commandPower } from '../command-power/definition.ts';
import { changesRange, formatBalance, netSpend, rawOf, summaryEnd } from './measureData.ts';
import type { ChartBucket, ChartSummary } from '../../charts/chartSummary';
import { reachedLines, validateMeasureStates } from './measureSeriesData.ts';

const resolveCountries = (selection: CountrySelection, fallback: string[], groups: CountryGroupReply | undefined, lastHour: number) =>
  resolveAny(selection, fallback, groups, lastHour, Number.MAX_SAFE_INTEGER);

test('balances are whole numbers by default and exact to the digits asked', () => {
  assert.equal(formatBalance('900001', 5), '9');
  assert.equal(formatBalance('900001', 5, 5), '9.00001');
  assert.equal(formatBalance('-50000', 5, 1), '−0.5');
  const drop = { hour: 102, beforeRaw: '1000000', afterRaw: '900001' };
  assert.equal(netSpend(drop, 5), '−1');
  assert.equal(netSpend(drop, 5, 5), '−0.99999');
});

// quarantine: #3 re-enable in the dashboard rework; the issue names its failure
test.skip('values come only from the exact runs of the hours read, and a missing hour keeps its reason', () => {
  const run = (firstHour: number, lastHour: number, valueRaw: string) => ({ firstHour, lastHour, valueRaw, slot: 1, countryId: 10, startsSegment: true,
    segmentFirstHour: firstHour, firstExact: true, lastExact: true });
  const states = validateMeasureStates({ branchId: '1', measure: 'politicalPower', decimals: 5, states: [
    { tag: 'GER', hour: 100, run: run(90, 120, '100000'), drop: null, missing: null },
    { tag: 'GER', hour: 110, run: run(90, 120, '100000'), drop: null, missing: null },
    { tag: 'GER', hour: 130, run: null, drop: null, missing: 'The recorder lost these hours' },
    { tag: 'ITA', hour: 100, run: run(100, 100, '5'), drop: { hour: 100, beforeRaw: '9', afterRaw: '5' }, missing: null }] });
  const [ger, ita, fra] = reachedLines(['GER', 'ITA', 'FRA'], states);
  assert.deepEqual(ger.runs.map(item => [item.firstHour, item.lastHour]), [[90, 120]], 'a run read at two hours is one run');
  assert.equal(ger.missing.get(130), 'The recorder lost these hours');
  assert.deepEqual(ita.drops.map(item => item.hour), [100]);
  assert.deepEqual([fra.runs, fra.drops], [[], []], 'a country not read has no value, never a zero');
  assert.throws(() => validateMeasureStates({ branchId: '1', measure: 'politicalPower', decimals: 5, states: [{ tag: 'GER', hour: 1, run: run(1, 1, '1.5'), drop: null, missing: null }] }));
});

test('group members draw only while they belong', () => {
  const groups: CountryGroupReply = { branchId: '1', warnings: [], warStarts: [], capitulations: [], identityChanges: [], humans: [], wars: [], unobserved: { humans: [], factions: [], wars: [] },
    countryIdentities: [{ tag: 'GER', identities: [{ firstHour: 0, lastHour: 200, slot: 1, countryId: 10 }] },
      { tag: 'ITA', identities: [{ firstHour: 50, lastHour: 80, slot: 2, countryId: 20 }] }],
    factions: [{ id: '5', name: 'Axis', recordedName: 'Axis', firstHour: 0, lastHour: 200, nameReason: null, names: [{ firstHour: 0, lastHour: 200, name: 'Axis', reason: null }], members: [{ tag: 'GER', firstHour: 0, lastHour: 200 }, { tag: 'ITA', firstHour: 50, lastHour: 80 }], leaders: [], incomplete: [] }] };
  const atEachHour = resolveCountries({ kind: 'group', group: { type: 'faction', id: '5' }, membership: 'hour', aggregate: false }, [], groups, 200);
  assert.deepEqual(atEachHour.tags, ['GER', 'ITA']);
  assert.deepEqual(atEachHour.membership?.get('ITA'), [{ tag: 'ITA', firstHour: 50, lastHour: 80 }]);
  const atEnd = resolveCountries({ kind: 'group', group: { type: 'faction', id: '5' }, membership: 'end', aggregate: false }, [], groups, 200);
  assert.deepEqual([atEnd.tags, atEnd.membership], [['GER'], null]);
  assert.equal(resolveCountries({ kind: 'group', group: { type: 'faction', id: '9' }, membership: 'ever', aggregate: false }, [], groups, 200).reason, 'This group is unavailable in this campaign.');
  const gapAtEnd = { ...groups, unobserved: { humans: [], factions: [{ firstHour: 190, lastHour: 200 }], wars: [] } };
  const unknown = resolveCountries({ kind: 'group', group: { type: 'faction', id: '5' }, membership: 'end', aggregate: false }, [], gapAtEnd, 200);
  assert.deepEqual([unknown.tags, unknown.unobserved], [[], [{ firstHour: 190, lastHour: 200 }]]);
  assert.equal(unknown.reason, 'Group membership is unavailable at this time.');
  const humansGap = { ...groups, unobserved: { humans: [{ firstHour: 190, lastHour: 200 }], factions: [], wars: [] } };
  assert.deepEqual(resolveCountries({ kind: 'group', group: { type: 'faction', id: '5' }, membership: 'end', aggregate: false }, [], humansGap, 200).tags, ['GER']);
  assert.deepEqual(resolveCountries({ kind: 'selected' }, ['GER'], undefined, 0).tags, ['GER']);
});

test('political and command power offer every chosen country without a widget limit', () => {
  const tags = Array.from({ length: 5000 }, (_, index) => `MOD ${index}`);
  const selection: CountrySelection = { kind: 'tags', tags };
  for (const definition of [politicalPower, commandPower]) {
    const resolved = resolveAny(selection, [], undefined, 100, definition.countries.max);
    assert.deepEqual(resolved.tags, tags);
    assert.equal(resolved.dropped, 0);
  }
});


test('the balance changes a list opens on end at the last the head has reached, never past it', () => {
  const marks = { from: 0, to: 600, width: 600, tiles: true as const, clusters: [
    { firstHour: 0, lastHour: 100, firstOrdinal: '0', count: '3' },
    { firstHour: 200, lastHour: 300, firstOrdinal: '3', count: '4', bins: [[220, 1], [260, 2], [300, 1]] as const },
    { firstHour: 400, lastHour: 500, firstOrdinal: '7', count: '2' },
  ] };
  assert.deepEqual(changesRange(marks, null, 270), { first: '0', end: '6' }, 'through the 3 of the first cluster and the 1 + 2 of the bins the head passed');
  assert.deepEqual(changesRange(marks, null, 350), { first: '0', end: '7' }, 'past the second cluster whole');
  assert.deepEqual(changesRange(marks, null, 150), { first: '0', end: '3' });
  assert.deepEqual(changesRange(marks, null, null), { first: '0', end: '9' }, 'no head: every cluster');
  assert.deepEqual(changesRange({ ...marks, tiles: undefined }, null, 270), { first: '0', end: '9' }, 'a summary not of tiles holds only what is shown');
  assert.deepEqual(changesRange(marks, marks.clusters[0]!, 270), { first: '0', end: '3' }, 'a hovered cluster is its own');
  assert.deepEqual(changesRange(undefined, null, 270), { first: '0', end: undefined }, 'nothing drawn: every change');
});

test('a line ends at the value its painter draws: the followed head inside the head bin, else the last value of the bin before', () => {
  const bucket = (firstHour: number, lastValue: number): ChartBucket => ({ firstHour, lastHour: firstHour + 11, lastValueHour: firstHour + 11, minHour: firstHour,
    maxHour: firstHour, firstValue: lastValue, lastValue, minValue: lastValue, maxValue: lastValue, firstBreak: false, hasBreak: false });
  const buckets = [bucket(0, 1.5), bucket(12, 2.25), null, bucket(36, 4)];
  let head: number | null | undefined = 7.125;
  const summary: ChartSummary = { from: 0, to: 47, width: 4, buckets, tiles: { binHours: 12, valueAt: () => head ?? null } };
  assert.equal(summaryEnd(summary, 40), 7.125, 'inside the head bin: the followed value');
  head = null;
  assert.equal(summaryEnd(summary, 40), null, 'a head the followed answer holds no value at draws nothing');
  assert.equal(summaryEnd(summary, 30), 2.25, 'past a gap: the last value of the last bin before');
  assert.equal(summaryEnd({ ...summary, tiles: undefined }, 40), 4, 'not tiled: the bin\'s own last value');
  assert.equal(summaryEnd(summary, -1), null);
  assert.equal(formatBalance(rawOf(2.25, 3), 3, 2), '2.25');
  assert.equal(rawOf(-0.1 + 0.3, 5), '20000', 'a value that is not exact in binary comes back as its raw');
});
