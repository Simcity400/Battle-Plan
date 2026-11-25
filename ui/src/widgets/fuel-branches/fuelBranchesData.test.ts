import test from 'node:test';
import assert from 'node:assert/strict';
import { branchName, percentText, ratioAxis, validateFuelBranches } from './fuelBranchesData.ts';
import type { FuelBranchesReply } from '../../generated/dashboard';

const reply = (): FuelBranchesReply => ({
  tag: 'AAA',
  scale: 100000,
  consumerNames: ['Land', null, 'Sea', null],
  consumerIcons: ['http://artwork.localhost/army.png', null, 'http://artwork.localhost/navy.png', null],
  consumerIconReasons: [null, 'missing air icon', null, 'extra consumer'],
  fuelIcon: 'http://artwork.localhost/fuel.png',
  fuelIconReason: null,
  readHour: 10,
  missing: null,
  warnings: [],
  consumers: [
    { index: 0, requested: '500000', received: '250000', priority: 1 },
    { index: 3, requested: '100', received: '900', priority: 2 },
  ],
});

test('branchName falls back when content names no consumer', () => {
  const validated = validateFuelBranches(reply());
  assert.equal(branchName(validated.consumerNames, 0), 'Land');
  assert.equal(branchName(validated.consumerNames, 3), 'Branch name unavailable');
});

test('a share axis reaches every share shown in round steps, and percents keep two places only when needed', () => {
  assert.deepEqual(ratioAxis(40), { top: 100, ticks: [0, 25, 50, 75, 100] });
  assert.equal(ratioAxis(900).top, 1000);
  assert.equal(percentText(50), '50%');
  assert.equal(percentText(33.3333), '33.33%');
});

test('a malformed or out of order reply is refused', () => {
  const bad = reply();
  bad.consumers.reverse();
  assert.throws(() => validateFuelBranches(bad));
  assert.throws(() => validateFuelBranches({ ...reply(), missing: 3 }));
  assert.throws(() => validateFuelBranches({ ...reply(), fuelIcon: 12 }));
});
