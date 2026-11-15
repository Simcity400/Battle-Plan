import assert from 'node:assert/strict';
import test from 'node:test';
import { factTexts, peopleStructure, peopleValue, validatePeople } from './peopleData.ts';
const reply = { branchId: '1', tag: 'MOD', hour: 100, readHour: 99, manpower: [{ label: 'Manpower', value: '9223372036854775807', unit: 'people' }], politics: [{ label: 'Election', value: 'Allowed', unit: 'text' }], states: [{ state: 1, available: -9, locked: -17, total: 512, effective: '2500', recruitable: 12 }], exiles: [], warnings: [] };
test('manpower validates every fact and preserves missing and signed state operands', () => {
  assert.equal(validatePeople(reply), reply);
  assert.throws(() => validatePeople({ ...reply, manpower: [{ label: 3, value: null, unit: 'people' }] }));
  assert.throws(() => validatePeople({ ...reply, manpower: [{ label: 'x', value: '1.2', unit: 'people' }] }));
  assert.throws(() => validatePeople({ ...reply, states: [{ ...reply.states[0], effective: 'x' }] }));
});
test('manpower formats exact large integers and the applied ratio scaling', () => {
  assert.equal(peopleValue({ value: '9223372036854775807', unit: 'people' }), '9,223,372,036,854,775,807');
  assert.equal(peopleValue({ value: '250000', unit: 'ratio100' }), '2.50%');
  assert.equal(peopleValue({ value: '-2500', unit: 'ratio' }), '-2.50%');
  assert.equal(peopleValue({ value: '11', unit: 'dailyRatio' }), '0.011%');
  assert.equal(peopleValue({ value: '2599', unit: 'ratio' }), '2.59%');
  assert.equal(peopleValue({ value: null, unit: 'people' }), 'Not recorded');
});

test('exile cards require their independently optional deployed air crews', () => {
  const exile = { tag: 'EXL', host: 'HST', legitimacy: '0', manpower: 7, cap: 10, leaders: 0, deployedAir: 100, readHour: 99 };
  assert.equal(validatePeople({ ...reply, exiles: [exile] }).exiles[0].deployedAir, 100);
  assert.equal(validatePeople({ ...reply, exiles: [{ ...exile, deployedAir: null }] }).exiles[0].deployedAir, null);
  assert.throws(() => validatePeople({ ...reply, exiles: [{ ...exile, deployedAir: '100' }] }));
  assert.throws(() => validatePeople({ ...reply, exiles: [{ ...exile, deployedAir: undefined }] }));
});

test('a followed view renders when its facts or lists change shape and paints their values between', () => {
  const later = { ...reply, hour: 101, manpower: [{ label: 'Manpower', value: '12', unit: 'people' }] };
  assert.equal(peopleStructure(validatePeople(later), 'manpower'), peopleStructure(validatePeople(reply), 'manpower'));
  assert.equal(factTexts(later.manpower).get('Manpower'), '12');
  assert.notEqual(peopleStructure(validatePeople({ ...reply, exiles: [{ tag: 'EXL', host: null, legitimacy: null, manpower: null, cap: null, leaders: null, deployedAir: null, readHour: null }] }), 'politics'),
    peopleStructure(validatePeople(reply), 'politics'));
});
