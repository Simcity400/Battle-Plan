import assert from 'node:assert/strict';
import { test } from 'node:test';
import { garrisonCell, occupiedAt, occupationTooltip, percent, percentCell, validateOccupiedStatesReply } from './occupiedStatesData.ts';
import type { OccupiedStatesReply, OccupiedValues } from '../../generated/dashboard';

const value: OccupiedValues = { name: 'Mod state', nameMissing: null, owner: 'Owner with spaces', occupier: 'Dynamic tag', occupied: 'Original tag',
  resistanceRaw: '9007199254740993', complianceRaw: '0', resistanceSpeedRaw: '-1', complianceSpeedRaw: '10000',
  garrisonStrengthRaw: null, requiredManpower: 0, garrisonRequiredRaw: null, garrisonAlert: false, garrisonRed: null, assignedManpower: null,
  garrisonMissing: 'Garrison row missing', law: 'extra_mod_law', lawMissing: null };
const reply: OccupiedStatesReply = { branchId: '1', decimals: 5,
  states: [{ state: 2000, runs: [{ firstHour: 10, lastHour: 33, value }, { firstHour: 40, lastHour: 50, value }] }],
  observed: [{ firstHour: 10, lastHour: 20, readHour: 10, complete: true }, { firstHour: 21, lastHour: 33, readHour: 21, complete: true }, { firstHour: 40, lastHour: 50, readHour: 40, complete: true }],
  laws: [{ token: 'extra_mod_law', name: 'Overridden law name', icon: null, missing: 'Mod sprite missing' }], strengthMissing: 'Equipment inputs missing', warnings: [] };

test('validation accepts exact mod values and refuses malformed nested payloads', () => {
  assert.equal(validateOccupiedStatesReply(reply), reply);
  for (const data of [null, {}, { ...reply, states: [null] }, { ...reply, observed: [null] }, { ...reply, decimals: 4 },
    { ...reply, states: [{ state: 1, runs: [{ firstHour: 0, lastHour: 1, value: { ...value, resistanceRaw: '9223372036854775808' } }] }] },
    ...[{ garrisonAlert: 'yes' }, { garrisonRed: 1 }, { assignedManpower: 0.5 }].map(fields => ({ ...reply, states: [{ state: 1, runs: [{ firstHour: 10, lastHour: 20, value: { ...value, ...fields } }] }] })),
    { ...reply, observed: [{ firstHour: 10, lastHour: 20, readHour: 11, complete: true }] },
    { ...reply, observed: [{ firstHour: 10, lastHour: 33, readHour: 9, complete: true }, { firstHour: 40, lastHour: 50, readHour: 40, complete: true }] },
    { ...reply, observed: [{ firstHour: 10, lastHour: 33, readHour: 10, complete: true }, { firstHour: 40, lastHour: 50, readHour: 10, complete: true }] },
    { ...reply, laws: [...reply.laws, ...reply.laws] },
    { ...reply, states: [{ state: 1, runs: [{ firstHour: 10, lastHour: 50, value }] }] },
    { ...reply, observed: [{ firstHour: 10, lastHour: 20, readHour: 10, complete: true }, { firstHour: 21, lastHour: 33, readHour: 9, complete: true }] },
    { ...reply, states: [{ state: 1, runs: [{ firstHour: 10, lastHour: 20, value }, { firstHour: 20, lastHour: 30, value }] }] }]) {
    assert.throws(() => validateOccupiedStatesReply(data), /malformed/);
  }
});

test('playback is an exact lookup; missing hours and known empty inventories differ', () => {
  assert.equal(occupiedAt(reply, 25, null, 'occupier').rows[0].value.resistanceRaw, '9007199254740993');
  assert.equal(occupiedAt(reply, 25, null, 'occupier').rows[0].hour, 21);
  assert.equal(occupiedAt(reply, 35, null, 'occupier').observation, null);
  assert.equal(occupiedAt({ ...reply, states: [] }, 25, null, 'occupied').observation?.complete, true);
  assert.equal(occupiedAt(reply, 9, null, 'occupied').rows.length, 0);
});

test('occupied filter uses the original tag, not legal owner; tags remain opaque', () => {
  assert.equal(occupiedAt(reply, 25, new Set(['Original tag']), 'occupied').rows.length, 1);
  assert.equal(occupiedAt(reply, 25, new Set(['Owner with spaces']), 'occupied').rows.length, 0);
  assert.equal(occupiedAt(reply, 25, new Set(['Dynamic tag']), 'occupier').rows.length, 1);
});

test('exact labels distinguish missing from zero and date equal consecutive reads', () => {
  assert.equal(percent('10000000'), '100%'); assert.equal(percent('9999999'), '99.99999%');
  const row = occupiedAt(reply, 25, null, 'occupier').rows[0];
  const label = (tag: string) => `Recorded name of ${tag}`;
  const tip = occupationTooltip(reply, row, 25, label);
  assert.equal(tip.rows.find(item => item.label === 'Occupied country')?.value, 'Recorded name of Original tag');
  assert.equal(tip.rows.find(item => item.label === 'Resistance speed')?.value, '−0.00001 pp/day');
  assert.equal(tip.rows.find(item => item.label === 'Compliance speed')?.value, '+0.1 pp/day');
  assert.equal(tip.rows.find(item => item.label === 'Required manpower')?.value, '0');
  assert.equal(tip.rows.find(item => item.label === 'Required garrison amount')?.value, '—');
  assert.match(tip.note!, /Read .*occupation values are read once a day/);
  assert.equal(occupationTooltip(reply, row, 21, label).note, undefined);
  const known = occupationTooltip(reply, { ...row, value: { ...row.value, garrisonStrengthRaw: '0' } }, 25, label);
  assert.equal(known.rows.find(item => item.label === 'Garrison strength')?.value, '0%');
  assert.equal(known.description, undefined);
});

test('garrison text is the native amount; missing/zero hide bars, failure is !, known strength only drives the bar', () => {
  assert.deepEqual(garrisonCell(value, true), { text: '-', bar: null, red: false });
  assert.deepEqual(garrisonCell({ ...value, garrisonRequiredRaw: '0', garrisonStrengthRaw: '0' }, true), { text: '-', bar: null, red: false });
  assert.deepEqual(garrisonCell({ ...value, garrisonRequiredRaw: '-1', garrisonAlert: true }, true), { text: '!', bar: 0n, red: true });
  assert.deepEqual(garrisonCell({ ...value, garrisonRequiredRaw: '123999', garrisonStrengthRaw: '0', garrisonRed: true }, true), { text: '1.23', bar: 0n, red: true });
  assert.deepEqual(garrisonCell({ ...value, garrisonRequiredRaw: '100000' }, true), { text: '1.00', bar: null, red: false });
  assert.deepEqual(garrisonCell({ ...value, garrisonRequiredRaw: '123999', garrisonStrengthRaw: '0', garrisonRed: false }, false), { text: '1.23', bar: null, red: false });
});

test('native occupied-state percentage cells round half away from zero, then clamp to 0..100', () => {
  for (const [raw, text] of [
    ['4133540', '41%'], ['940099', '9%'], ['4149999', '41%'], ['4150000', '42%'],
    ['49999', '0%'], ['50000', '1%'], ['0', '0%'], ['9999999', '100%'],
    ['10000000', '100%'], ['10100000', '100%'], ['-50000', '0%'],
  ]) assert.equal(percentCell(raw), text, raw);
});

test('compact native cells retain exact resistance, compliance, speeds and garrison in the tooltip', () => {
  const shown = { ...value, resistanceRaw: '4133540', complianceRaw: '940099',
    resistanceSpeedRaw: '12345', complianceSpeedRaw: '-999', garrisonRequiredRaw: '123999' };
  assert.equal(percentCell(shown.resistanceRaw), '41%');
  assert.equal(percentCell(shown.complianceRaw), '9%');
  assert.equal(garrisonCell(shown, true).text, '1.23');
  const tip = occupationTooltip(reply, { key: '2000', state: 2000, hour: 21, value: shown }, 25, tag => tag);
  const text = (label: string) => tip.rows.find(row => row.label === label)?.value;
  assert.equal(text('Resistance'), '41.3354%');
  assert.equal(text('Compliance'), '9.40099%');
  assert.equal(text('Resistance speed'), '+0.12345 pp/day');
  assert.equal(text('Compliance speed'), '−0.00999 pp/day');
  assert.equal(text('Required garrison amount'), '1.23999');
});
