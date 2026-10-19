import { test } from 'node:test';
import assert from 'node:assert/strict';
import { intelTooltip, intelValues, validateIntelDetails } from './intelData.ts';

test('intel section retains derived pair ranges, reserved slots and named refusals', () => {
  const first = { key: '1:26:-7', group: 'Player estimates', label: 'Divisions', viewer: 17, target: 5000, supplyingHour: null, values: [{ name: 'Estimated range', value: '3 – 9' }], explanations: [{ name: 'Unlock', value: 'Viewer is the target country' }], unavailable: null };
  const second = { ...first, key: '1:26:2', values: [{ name: 'Estimated range', value: '?' }], unavailable: 'CountryIntel viewer 17 / target 5000 not recorded' };
  const reply = { viewer: 17, hour: 7, country: 5000, countries: [{ id: 5000, tag: 'MOD' }], entries: [first, second], warnings: [] };
  assert.equal(validateIntelDetails(reply), reply);
  assert.match(JSON.stringify(intelTooltip(first)), /3 – 9/);
  assert.match(intelTooltip(second).note!, /Explanation unavailable: CountryIntel/);
  assert.throws(() => validateIntelDetails({ ...reply, entries: [first, first] }), /malformed/);
  assert.throws(() => validateIntelDetails({ ...reply, entries: [{ ...first, explanations: [{ name: 'bad', value: 7 }] }] }), /malformed/);
});



test('intel section exposes reserved raw-zero holders and validates the viewing country', () => {
  const reply = { hour: 7, viewer: 17, country: 0, countries: [{ id: 0, tag: 'Reserved slot 0 / raw country 0 holders' }], entries: [], warnings: [] };
  assert.equal(validateIntelDetails(reply), reply);
  assert.throws(() => validateIntelDetails({ ...reply, viewer: '17' }), /malformed/);
});

test('retained fuel ranges show the supplying date in the table and hover', () => {
  const hour = 43_800_000 + 1936 * 365 * 24 + 17;
  const row = { key: 'fuel', group: 'Player estimates', label: 'Fuel', viewer: 17, target: 17,
    supplyingHour: hour, values: [{ name: 'Estimated range', value: '1000 - 1000' }], explanations: [], unavailable: null };
  const reply = { viewer: 17, hour: hour + 1, country: 17, countries: [], entries: [row], warnings: [] };
  assert.equal(validateIntelDetails(reply), reply);
  const date = intelValues(row).find(value => value.name === 'As of')!;
  assert.match(date.value, /1 Jan 1936/);
  assert.match(date.value, /18:00/);
  assert.match(JSON.stringify(intelTooltip(row)), /18:00/);
  assert.throws(() => validateIntelDetails({ ...reply, entries: [{ ...row, supplyingHour: hour + 2 }] }), /malformed/);
  assert.throws(() => validateIntelDetails({ ...reply, entries: [{ ...row, supplyingHour: '17' }] }), /malformed/);
});
