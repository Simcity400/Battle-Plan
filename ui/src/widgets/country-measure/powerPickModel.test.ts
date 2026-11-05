import test from 'node:test';
import assert from 'node:assert/strict';
import type { CountryStabilityReply } from '../../generated/dashboard';
import { powerMaximaModel } from './powerPickModel.ts';

const maxima = (): CountryStabilityReply => ({ branchId: 'branch', decimals: 5, warnings: [], series: [{
  tag: 'recorded actor', stability: [], warSupport: [], recorded: true, breakdown: null,
  maxCommandPower: [{ firstHour: 24, lastHour: 48, valueRaw: '10000000', startsSegment: true }],
  carried: [{ firstHour: 25, lastHour: 48, readHour: 24 }],
}] });
test('maximum ownership excludes content presentation but retains numeric scale, values and supplying dates', () => {
  const original = maxima();
  const refreshed = { ...maxima(), warnings: ['Changed content warning'], series: maxima().series.map(row => ({ ...row, recorded: false })) };
  assert.equal(powerMaximaModel(refreshed).key, powerMaximaModel(original).key);
  for (const changed of [
    { ...maxima(), decimals: 4 },
    { ...maxima(), series: maxima().series.map(row => ({ ...row, maxCommandPower: [{ firstHour: 24, lastHour: 48, valueRaw: '9000000', startsSegment: true }] })) },
    { ...maxima(), series: maxima().series.map(row => ({ ...row, carried: [{ firstHour: 25, lastHour: 48, readHour: 23 }] })) },
  ]) assert.notEqual(powerMaximaModel(changed).key, powerMaximaModel(original).key);
});
