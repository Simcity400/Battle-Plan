import test from 'node:test';
import assert from 'node:assert/strict';
import { channelIndex, statisticName, validateAirStatistics } from './airStatisticsData.ts';

const reply = (regions: { region: string; name: string | null; missing: string | null }[], missing: string | null = null) => validateAirStatistics({
  readHour: 48, missing, warnings: [], theatres: [{ id: '7', name: 'West', regions: ['10'] }], theatresMissing: null,
  channels: [{ id: '2', ours: 'Ships bombed', enemy: 'Ships sunk', divisor: 100000 }, { id: '3', ours: 'Fighters lost:', enemy: 'Fighters destroyed', divisor: 100000 }],
  regions,
});

test('a channel name drops the trailing colon the game writes', () => {
  assert.equal(statisticName(reply([]).channels[1].ours), 'Fighters lost');
});

test('statistics missing at the hour hold no rows, and malformed replies are refused', () => {
  assert.equal(reply([], 'Air statistics are not recorded at this hour').regions.length, 0);
  assert.throws(() => reply([{ region: '1', name: 'A', missing: null }], 'not recorded'), /malformed/);
  assert.throws(() => reply([{ region: '1', name: 'A', missing: 'both' }]), /malformed/);
  assert.equal(channelIndex('11'), 11);
  assert.equal(channelIndex('12'), null);
});

test('the theatres at the hour validate with the reply, and malformed ones refuse it', () => {
  assert.deepEqual(reply([]).theatres, [{ id: '7', name: 'West', regions: ['10'] }]);
  const base = { readHour: 48, missing: null, warnings: [], channels: [], regions: [] };
  assert.equal(validateAirStatistics({ ...base, theatres: null, theatresMissing: 'Air-theatre owner is unresolved' }).theatresMissing, 'Air-theatre owner is unresolved');
  assert.throws(() => validateAirStatistics({ ...base, theatres: [{ id: 7, name: 'West', regions: [] }], theatresMissing: null }), /malformed/);
  assert.throws(() => validateAirStatistics({ ...base, theatresMissing: null }), /malformed/);
});

test('the bars butterfly keeps the region order the product gives, puts ours left and the enemy right, and a word not recorded is missing, never zero', async () => {
  const { airStatisticsProjection } = await import('./airStatisticsProjection.ts');
  const { createBoundedReadView, displayedHour } = await import('../../families/boundedView.ts');
  const { valuesOf } = await import('../../test-support/liveValues.ts');
  const { liveKey } = await import('../../charts/live.ts');
  const data = reply([{ region: '11', name: 'Baltic', missing: null }, { region: '10', name: 'North Sea', missing: null }]);
  const live = valuesOf(new Map([[liveKey('11', 'ours'), 250000], [liveKey('11', 'enemy'), 500000], [liveKey('10', 'ours'), 0]]), 'Not recorded at this hour');
  const view = createBoundedReadView({ hour: displayedHour(48), panels: new Map([['main', data]]), live: new Map([['main', live]]) });
  const model = airStatisticsProjection.project(view, { channel: '3', country: null, ratio: true });
  const rows = model.rows as unknown as readonly { key: string; left: { value: { value: { kind: string } } }; right: { value: { value: { kind: string } } }; readout?: { primary: string } }[];
  assert.deepEqual(rows.map(row => row.key), ['11', '10'], 'the product ranks; the client keeps the order');
  assert.deepEqual([model.leftHeader, model.rightHeader], ['Fighters lost', 'Fighters destroyed']);
  assert.equal(rows[0]!.readout?.primary, '2.00');
  assert.deepEqual([rows[1]!.left.value.value.kind, rows[1]!.right.value.value.kind], ['zero', 'notRecorded']);
});
