import test from 'node:test';
import assert from 'node:assert/strict';
import { efficiencyText, missionRowsAt, missionTooltip, openWings, regionRows, shownMissions, theatreRegions, validateAirMissions, validateAirWings } from './airMissionsData.ts';
import { CLOSED, reduceDrill } from '../../charts/drill/drillState.ts';

const reply = validateAirMissions({
  window: { firstHour: 0, lastHour: 10, known: [{ firstHour: 0, lastHour: 8 }], reads: [[0, 8, 0]], series: [
    { name: 'GER@7|10|m0', runs: [[0, 8, '24', true]] },
    { name: 'GER@7|10|m2', runs: [[0, 4, '24', true], [5, 8, '0', false]] },
    { name: 'GER@7|10|wings', runs: [[0, 8, '1', true]] },
    { name: 'GER@7|11|m1', runs: [[5, 8, '50', true]] },
    { name: 'GER@7|11|wings', runs: [[5, 8, '2', true]] },
    { name: 'ITA@8|10|m0', runs: [[0, 8, '9', true]] },
  ] },
  missions: [{ id: '0', name: 'Air superiority', icon: { key: 'GFX_strip', frame: 0 } }, { id: '1', name: 'Close air support', icon: { key: 'GFX_strip', frame: 1 } },
    { id: '2', name: 'Interception', icon: null }, { id: '3', name: 'Port strike', icon: { key: 'GFX_strip', frame: 8 } }],
  regions: [{ id: '10', name: 'North Sea' }, { id: '11', name: 'Rhineland' }],
  theatres: [{ id: 't', name: 'West', regions: ['11'] }],
  warnings: [],
});

test('a wing on two missions counts in each mission but once in wings, and a zero stays known', () => {
  const late = missionRowsAt(reply, 'GER', 6)!;
  const north = late.rows.find(row => row.region === '10')!;
  assert.deepEqual([...north.planes], [['0', 24n], ['2', 0n]]);
  assert.equal(north.wings, 1n);
  assert.deepEqual(shownMissions(reply.missions, late.rows, []).map(mission => mission.id), ['0', '1', '2']);
  assert.deepEqual(shownMissions(reply.missions, late.rows, ['1']).map(mission => mission.id), ['1']);
  assert.match(missionTooltip(north, reply.missions, '1 Jan').rows.map(row => `${row.label} ${row.value}`).join('|'), /Air superiority 24 planes\|Interception 0 planes\|Wings 1/);
});

test('rows follow the country and hour; unrecorded hours are unknown', () => {
  assert.deepEqual(missionRowsAt(reply, 'GER', 2)!.rows.map(row => row.region), ['10']);
  assert.deepEqual(missionRowsAt(reply, 'ITA', 2)!.rows.map(row => row.wings), [null]);
  assert.equal(missionRowsAt(reply, 'GER', 9), null);
});

test('theatres narrow the regions', () => {
  const rows = missionRowsAt(reply, 'GER', 6)!.rows;
  assert.deepEqual(regionRows(rows, theatreRegions(reply.theatres, ['t'])).map(row => row.region), ['11']);
  assert.equal(theatreRegions(reply.theatres, []), null);
});

test('wing details validate and efficiency keeps the game percent scale', () => {
  const wings = validateAirWings({ hour: 5, readHour: 5, region: 10, warnings: [], wings: [
    { id: '1', name: null, type: 'Fighter', planes: 24, missions: ['0', '2'], efficiency: '87500', refusals: [] }] });
  assert.equal(wings.wings[0].planes, 24);
  assert.equal(efficiencyText('87500'), '87.5%');
  assert.equal(efficiencyText(null), '—');
  assert.throws(() => validateAirWings({ hour: 5, readHour: 5, region: 10, warnings: [], wings: [{ id: 1 }] }));
  // The backend serializes the drill's region as its u32.
  assert.throws(() => validateAirWings({ hour: 5, readHour: 5, region: '10', warnings: [], wings: [] }), /malformed/u);
});

test('a mission icon is a strip frame or none, and anything else refuses the reply', () => {
  const base = { window: { firstHour: 0, lastHour: 0, known: [], reads: [], series: [] }, regions: [], warnings: [] };
  assert.equal(validateAirMissions({ ...base, missions: [{ id: '0', name: 'A', icon: null }] }).missions[0].icon, null);
  const recorded = { key: 'GFX_strip', frame: 2, recorded: 'GFX_x' };
  assert.deepEqual(validateAirMissions({ ...base, missions: [{ id: '0', name: 'A', icon: recorded }] }).missions[0].icon, recorded);
  for (const icon of [undefined, 'GFX_strip', { key: 'GFX_strip', frame: -1 }, { key: 'GFX_strip', frame: 0.5 },
    { key: 'GFX_strip', frame: 0, recorded: 5 }, { key: 'GFX_strip', frame: 0, recorded: '' }])
    assert.throws(() => validateAirMissions({ ...base, missions: [{ id: '0', name: 'A', icon }] }), /malformed/u);
});

test('rows held from another hour open their wings pinned there, however near the playhead', () => {
  // Held rows at 100 while paused or playing at 101: never the playhead's wings.
  for (const following of [true, false]) assert.deepEqual(reduceDrill(CLOSED, openWings(100, 101, following)), { breakdown: 'wings', follow: false, hour: 100 });
  assert.equal(reduceDrill(CLOSED, openWings(101, 101, true)).follow, true, "the playhead's own rows follow it");
});
