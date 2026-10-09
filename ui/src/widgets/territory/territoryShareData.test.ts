import assert from 'node:assert/strict';
import test from 'node:test';
import { raceSeriesIndex } from '../../charts/race/series.ts';
import type { TerritoryReply } from '../../generated/dashboard';
import { groupShareReason, resolveCountries } from '../../dashboard/groups.ts';
import { OPTIONS } from './definition.ts';
import { territoryShareReading, territoryWorldModel } from './territoryShareData.ts';

const look = (key: string) => ({ id: key, label: key, colour: '#797979' });
const rest = { id: 'rest', label: 'Rest of world', colour: '#888888' };
const span = { firstHour: 1, lastHour: 20 };
const reply = (values: (number | null)[] = [100, 200, 300, 400, 500, 600, 700, 800]): TerritoryReply =>
  ({ branchId: 'branch', series: [], worldTotals: [{ ...span, startsSegment: false, values }], areaUnit: 'map cells', areaReason: null, warnings: [] });
const series = raceSeriesIndex([{ tag: 'a', runs: [{ ...span, value: 30 }] }]);

test('Share is the third view, keeps Race as default, and exposes no line axis settings', () => {
  const view = OPTIONS.find(option => option.key === 'view')!;
  assert.equal(view.kind, 'choice');
  if (view.kind !== 'choice') throw new Error('View must be a choice.');
  assert.equal(view.default, 'race');
  assert.deepEqual(view.choices.map(choice => choice.value), ['race', 'lines', 'share']);
  for (const option of OPTIONS.filter(option => option.section === 'axes' || ['playback', 'afterPlayhead', 'windowDays'].includes(option.key))) {
    assert.equal(option.visibleWhen?.({ view: 'share' }), false);
  }
});

test('a top-country reply subtracts its slices from independently recorded world totals for each measure and rule', () => {
  for (const [measure, controlled, owned] of [['states', 100, 500], ['provinces', 200, 600], ['victoryPoints', 300, 700], ['area', 400, 800]] as const) {
    for (const [rule, total] of [['controlled', controlled], ['owned', owned]] as const) {
      const reading = territoryShareReading(series, territoryWorldModel(reply(), measure, rule), 10, 10, look, rest);
      assert.equal(reading.items[0].value, 30);
      assert.equal(reading.items[1].value, total - 30);
      assert.equal(reading.unavailableReason, null);
    }
  }
});

test('missing world hours stay gaps and a later complete reading starts a fresh stretch', () => {
  const data = reply();
  data.worldTotals = [{ firstHour: 1, lastHour: 5, startsSegment: false, values: [100, 200, 300, 400, 500, 600, 700, 800] },
    { firstHour: 6, lastHour: 7, startsSegment: false, values: [null, 200, 300, 400, 500, 600, 700, 800] },
    { firstHour: 8, lastHour: 20, startsSegment: false, values: [100, 200, 300, 400, 500, 600, 700, 800] }];
  const world = territoryWorldModel(data, 'states', 'controlled');
  assert.equal(territoryShareReading(series, world, 6, 10, look, rest).items.length, 0);
  assert.notEqual(territoryShareReading(series, world, 5, 10, look, rest).stretch,
    territoryShareReading(series, world, 8, 10, look, rest).stretch);
  assert.equal(territoryShareReading(series, territoryWorldModel(data, 'provinces', 'controlled'), 6, 10, look, rest).unavailableReason, null);
});

test('unknown factions outside the chosen top N and overlapping fixed rosters refuse the whole share', () => {
  const world = territoryWorldModel(reply(), 'states', 'controlled');
  const incomplete = territoryShareReading(series, world, 10, 1, look, rest, { recorded: [], disjoint: [span] });
  assert.equal(incomplete.items.length, 0);
  assert.equal(incomplete.missingLabel, 'Not recorded');
  const overlap = territoryShareReading(series, world, 10, 1, look, rest, { recorded: [span], disjoint: [] });
  assert.equal(overlap.items.length, 0);
  assert.equal(overlap.missingLabel, 'Share unavailable');
  // Even though the known selected total is only 30 of 100, fixed rosters overlap.
  assert.match(overlap.unavailableReason!, /overlap/);
});

test('an identity change without a missing world hour gives the slice a new recorded stretch', () => {
  const model = raceSeriesIndex([{ tag: 'opaque tag', runs: [{ firstHour: 1, lastHour: 9, value: 30 },
    { firstHour: 10, lastHour: 20, value: 30, breakBefore: true }] }]);
  const world = territoryWorldModel(reply(), 'states', 'controlled');
  const before = territoryShareReading(model, world, 9, 10, look, rest);
  const after = territoryShareReading(model, world, 10, 10, look, rest);
  assert.equal(before.stretch, after.stretch);
  assert.notEqual(before.items[0].path[0].stretch, after.items[0].path[0].stretch);
});

test('a dissolved faction selected as a summed Group retains its last-roster share during its lifetime', () => {
  const reading = territoryShareReading(series, territoryWorldModel(reply(), 'states', 'controlled'),
    10, 1, look, rest, { recorded: [span], disjoint: [span] });
  assert.equal(reading.unavailableReason, null);
  assert.equal(reading.items[0].value, 30);
  assert.equal(reading.items[1].value, 70);
});

test('a missing fixed choice cannot disappear into Rest of world even with a complete world read', () => {
  const reading = territoryShareReading(series, territoryWorldModel(reply(), 'states', 'controlled'),
    10, 10, look, rest, undefined, undefined, ['a', 'missing-country']);
  assert.equal(reading.items.length, 0);
  assert.equal(reading.missingLabel, 'Not recorded');
  assert.match(reading.unavailableReason!, /not recorded/);
});

test('a complete world and one known smart-group member cannot fill an incomplete roster into Rest of world', () => {
  const groups = { branchId: 'branch', humans: [{ tag: 'a', ...span }], factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [], warnings: [],
    countryIdentities: [{ tag: 'a', identities: [{ ...span, slot: 1, countryId: 10 }] }],
    unobserved: { humans: [{ firstHour: 10, lastHour: 10 }], factions: [], wars: [] } };
  const choice = { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: true } as const;
  const resolved = resolveCountries(choice, [], groups, span.lastHour, Infinity);
  const world = territoryWorldModel(reply(), 'states', 'controlled');
  const reading = territoryShareReading(series, world, 10, 10, look, rest, undefined, groupShareReason(resolved, choice.membership, 10));
  assert.equal(reading.items.length, 0, 'a known 30 of 100 cannot be presented as a complete 30 + 70 share');
  assert.equal(reading.missingLabel, 'Not recorded');
  assert.match(reading.unavailableReason!, /members.*not recorded/);
  assert.equal(groupShareReason(resolved, 'hour', 9), null);
  assert.notEqual(groupShareReason(resolved, 'ever', 9), null);
});

test('5,000 countries retain exact selected slices, the remainder and the full selection', () => {
  const lines = Array.from({ length: 5_000 }, (_, index) => ({ tag: `tag-${index}`, runs: [{ ...span, value: 5_000 - index }] }));
  const model = raceSeriesIndex(lines);
  const world = territoryWorldModel(reply(Array(8).fill(12_502_500)), 'states', 'controlled');
  const reading = territoryShareReading(model, world, 10, 30, look, rest);
  assert.deepEqual(reading.items.map(item => [item.path[0].id, item.value, item.exact]), [
    ...lines.slice(0, 30).map(line => [line.tag, line.runs[0].value, BigInt(line.runs[0].value)]),
    [rest.id, 12_352_935, 12_352_935n],
  ]);
  const all = territoryShareReading(model, world, 10, Number.MAX_SAFE_INTEGER, look, rest);
  assert.deepEqual(all.items.map(item => [item.path[0].id, item.value, item.exact]), [
    ...lines.map(line => [line.tag, line.runs[0].value, BigInt(line.runs[0].value)]), [rest.id, 0, 0n],
  ]);
});
