import test from 'node:test';
import assert from 'node:assert/strict';
import type { CountryGroupReply, MeasureSeries } from '../../generated/dashboard';
import { segments } from '../../charts/series.ts';
import { clipRuns, isMember, resolveCountries as resolveAny } from '../../dashboard/groups.ts';
import type { CountrySelection } from '../../dashboard/model';
import { formatBalance, MAX_COUNTRIES, nearestDrop, netSpend, rawAt, toRuns, validateMeasureReply } from './measureData.ts';

const resolveCountries = (selection: CountrySelection, fallback: string[], groups: CountryGroupReply | undefined, lastHour: number) =>
  resolveAny(selection, fallback, groups, lastHour, MAX_COUNTRIES);

const SERIES: MeasureSeries = {
  tag: 'GER',
  runs: [{ firstHour: 100, lastHour: 101, valueRaw: '1000000', startsSegment: true }, { firstHour: 102, lastHour: 102, valueRaw: '900001', startsSegment: false },
    { firstHour: 110, lastHour: 120, valueRaw: '-50000', startsSegment: true }],
  drops: [{ hour: 102, beforeRaw: '1000000', afterRaw: '900001' }],
};

test('replies are validated, including run order', () => {
  assert.equal(validateMeasureReply({ branchId: '1', decimals: 5, series: [SERIES], warnings: [] }).series[0].tag, 'GER');
  for (const bad of [null, { branchId: '1', decimals: 5, series: [{ ...SERIES, runs: [...SERIES.runs].reverse() }], warnings: [] },
    { branchId: '1', decimals: 5, series: [{ ...SERIES, runs: [{ firstHour: 1, lastHour: 2, valueRaw: '1.5' }] }], warnings: [] },
    { branchId: '1', decimals: 99, series: [], warnings: [] }]) {
    assert.throws(() => validateMeasureReply(bad));
  }
});

test('values are exact at every recorded hour and missing between runs', () => {
  assert.equal(rawAt(SERIES, 101), '1000000');
  assert.equal(rawAt(SERIES, 105), null);
  assert.equal(formatBalance(rawAt(SERIES, 102)!, 5), '9.00');
  assert.equal(formatBalance(rawAt(SERIES, 115)!, 5), '−0.50');
  assert.deepEqual(toRuns(SERIES, 5).map(run => run.value), [10, 9.00001, -0.5]);
  assert.equal(netSpend(SERIES.drops[0], 5), '−1.00');
  assert.equal(nearestDrop(SERIES.drops, 99, 5)?.hour, 102);
  assert.equal(nearestDrop(SERIES.drops, 90, 5), null);
});

test('group members draw only while they belong, and are capped at the widget limit', () => {
  const runs = toRuns(SERIES, 5);
  assert.deepEqual(clipRuns(runs, [{ tag: 'GER', firstHour: 101, lastHour: 112 }]).map(run => [run.firstHour, run.lastHour]), [[101, 101], [102, 102], [110, 112]]);
  const groups: CountryGroupReply = { branchId: '1', warnings: [], warStarts: [], capitulations: [], identityChanges: [], humans: [], wars: [], unobserved: { humans: [], factions: [], wars: [] },
    factions: [{ id: '5', name: 'Axis', firstHour: 0, lastHour: 200, members: [{ tag: 'GER', firstHour: 0, lastHour: 200 }, { tag: 'ITA', firstHour: 50, lastHour: 80 }] , incomplete: [] }] };
  const atEachHour = resolveCountries({ kind: 'group', group: { type: 'faction', id: '5' }, membership: 'hour', aggregate: false }, [], groups, 200);
  assert.deepEqual(atEachHour.tags, ['GER', 'ITA']);
  assert.deepEqual(atEachHour.membership?.get('ITA'), [{ tag: 'ITA', firstHour: 50, lastHour: 80 }]);
  const atEnd = resolveCountries({ kind: 'group', group: { type: 'faction', id: '5' }, membership: 'end', aggregate: false }, [], groups, 200);
  assert.deepEqual([atEnd.tags, atEnd.membership], [['GER'], null]);
  assert.equal(resolveCountries({ kind: 'group', group: { type: 'faction', id: '9' }, membership: 'ever', aggregate: false }, [], groups, 200).reason, 'This group is not in this campaign branch.');
  // Membership not recorded at the branch end is unknown, not an empty group; other histories' gaps do not count.
  const gapAtEnd = { ...groups, unobserved: { humans: [], factions: [{ firstHour: 190, lastHour: 200 }], wars: [] } };
  const unknown = resolveCountries({ kind: 'group', group: { type: 'faction', id: '5' }, membership: 'end', aggregate: false }, [], gapAtEnd, 200);
  assert.deepEqual([unknown.tags, unknown.unobserved], [[], [{ firstHour: 190, lastHour: 200 }]]);
  assert.match(unknown.reason ?? '', /not recorded/);
  const humansGap = { ...groups, unobserved: { humans: [{ firstHour: 190, lastHour: 200 }], factions: [], wars: [] } };
  assert.deepEqual(resolveCountries({ kind: 'group', group: { type: 'faction', id: '5' }, membership: 'end', aggregate: false }, [], humansGap, 200).tags, ['GER']);
  const many = Array.from({ length: MAX_COUNTRIES + 3 }, (_, index) => `T${index}`);
  const capped = resolveCountries({ kind: 'tags', tags: many }, [], undefined, 0);
  assert.deepEqual([capped.tags.length, capped.dropped], [MAX_COUNTRIES, 3]);
  assert.deepEqual(resolveCountries({ kind: 'selected' }, ['GER'], undefined, 0).tags, ['GER']);
});

test('a new country identity breaks the line even between consecutive hours', () => {
  const series: MeasureSeries = { tag: 'GER', drops: [], runs: [
    { firstHour: 100, lastHour: 101, valueRaw: '100000', startsSegment: true },
    { firstHour: 102, lastHour: 103, valueRaw: '200000', startsSegment: false },
    { firstHour: 104, lastHour: 105, valueRaw: '300000', startsSegment: true }] };
  assert.deepEqual(segments(toRuns(series, 5), 100, 105).map(segment => segment.length), [2, 1]);
});

test('membership limits which hours a group member shows', () => {
  const membership = [{ tag: 'ITA', firstHour: 50, lastHour: 80 }];
  assert.equal(isMember(membership, 60), true);
  assert.equal(isMember(membership, 102), false);
  assert.equal(isMember(undefined, 102), true);
});
