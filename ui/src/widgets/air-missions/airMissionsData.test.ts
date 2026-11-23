import test from 'node:test';
import assert from 'node:assert/strict';
import { liveKey } from '../../charts/live.ts';
import { NOT_FLOWN, choicesAt, flownBy, efficiencyText, missionRows, missionTooltip, missionValues, regionRows, shownMissions, theatreRegions, validateAirMissions, validateAirWings } from './airMissionsData.ts';

const reply = validateAirMissions({
  readHour: 6, missing: null, theatresMissing: null, warnings: [],
  missions: [{ id: '0', name: 'Air superiority', icon: { key: 'GFX_strip', frame: 0 } }, { id: '1', name: 'Close air support', icon: { key: 'GFX_strip', frame: 1 } },
    { id: '2', name: 'Interception', icon: null }],
  regions: [{ region: '10', name: 'North Sea', missing: null, missions: ['0', '2'] }, { region: '11', name: null, missing: 'Region 11 name unsupported', missions: ['1'] }],
  theatres: [{ id: 't', name: 'West', regions: ['11'] }],
});
const counts = new Map([[liveKey('10', 'm0'), 24], [liveKey('10', 'm2'), 0], [liveKey('10', 'wings'), 1], [liveKey('11', 'm1'), 50], [liveKey('11', 'wings'), 2]]);
const values = { get: (key: string) => counts.get(key), missing: () => 'Not in the answer' };

test('a region shows the missions it records; a zero stays known and an unflown mission says so', () => {
  const rows = missionRows(reply);
  assert.deepEqual(rows.map(row => [row.key, row.name, [...row.missions]]), [['10', 'North Sea', ['0', '2']], ['11', 'Region 11', ['1']]]);
  assert.deepEqual(shownMissions(reply.missions, rows, []).map(mission => mission.id), ['0', '1', '2']);
  assert.deepEqual(shownMissions(reply.missions, rows.slice(0, 1), ['1', '2']).map(mission => mission.id), ['2']);
  const shown = missionValues(values, rows);
  assert.equal(shown.get(liveKey('10', 'm2')), 0);
  assert.equal(shown.missing(liveKey('10', 'm1')), NOT_FLOWN);
  assert.equal(shown.missing(liveKey('12', 'wings')), 'Not in the answer');
  assert.deepEqual([liveKey('10', 'm1'), liveKey('10', 'm0'), liveKey('10', 'wings')].map(flownBy(rows)), [true, false, false], 'only a mission a region does not fly is empty');
  const tip = missionTooltip(rows[0], reply.missions, shown, '1 Jan').rows.map(row => `${row.label} ${row.value}`).join('|');
  assert.match(tip, /Air superiority 24 planes\|Interception 0 planes\|Wings 1\|As of 1 Jan/);
  assert.match(missionTooltip(rows[1], reply.missions, shown, null).rows.map(row => `${row.label} ${row.value}`).join('|'), /Name Region 11 name unsupported/);
});

test('theatres narrow the regions, and a chosen choice absent at the hour stays selectable', () => {
  const rows = missionRows(reply);
  assert.deepEqual(regionRows(rows, theatreRegions(reply.theatres, ['t'])).map(row => row.region), ['11']);
  assert.equal(theatreRegions(reply.theatres, []), null);
  assert.deepEqual(regionRows(rows, theatreRegions(null, ['t'])), []);
  assert.deepEqual(choicesAt([{ id: 't', name: 'West' }], ['t', 'u']).map(choice => choice.label), ['West', 'u · not at this date']);
});

test('replies validate, and anything else refuses them', () => {
  assert.throws(() => validateAirMissions({ ...reply, regions: [{ region: 10, name: null, missing: null, missions: [] }] }), /malformed/u);
  assert.throws(() => validateAirMissions({ ...reply, missions: [{ id: '0', name: 'A', icon: { key: 'GFX_strip', frame: -1 } }] }), /malformed/u);
  assert.throws(() => validateAirMissions({ ...reply, theatres: undefined }), /malformed/u);
  const wings = validateAirWings({ readHour: 5, region: 10, missing: null, warnings: [], wings: [
    { id: '1', name: null, type: 'Fighter', missions: ['0', '2'], efficiency: '87500', refusals: [] }] });
  assert.equal(wings.wings[0].type, 'Fighter');
  assert.throws(() => validateAirWings({ readHour: 5, region: '10', missing: null, warnings: [], wings: [] }), /malformed/u);
  assert.equal(efficiencyText('87500'), '87.5%');
  assert.equal(efficiencyText(null), '—');
});

