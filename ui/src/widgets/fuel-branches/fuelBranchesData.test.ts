import test from 'node:test';
import assert from 'node:assert/strict';
import { barWidths, branchesAt, fuelStructure, percentText, ratioAxis, validateFuelBranches } from './fuelBranchesData.ts';
import type { FuelBranchesReply } from '../../generated/dashboard';

const reply = (): FuelBranchesReply => ({
  tag: 'AAA', scale: 100000, consumerNames: ['Land', null, 'Sea', null], readHour: 10, missing: null, warnings: [],
  consumers: [
    { index: 0, requested: '500000', received: '250000', priority: 1 },
    { index: 3, requested: '100', received: '900', priority: 2 },
  ],
});

test('fuel by branch keeps the recorded consumers, unmet only below the request, and names a content-unnamed one', () => {
  const [land, fourth] = branchesAt(validateFuelBranches(reply()));
  assert.deepEqual([land.name, land.requested, land.received, land.unmet, land.priority], ['Land', 500000n, 250000n, 250000n, 1]);
  assert.deepEqual([fourth.index, fourth.name, fourth.unmet], [3, 'Branch name unavailable', 0n]);
});

test('bars share one scale, the largest amount shown, and a render follows only which rows and how they are drawn', () => {
  const widths = barWidths(branchesAt(reply()));
  assert.deepEqual(widths.get(0), { delivered: '50%', unmet: '50%' });
  assert.deepEqual(widths.get(3), { delivered: '0.02%', unmet: '0%' });
  const moved = reply();
  moved.consumers[0] = { ...moved.consumers[0], received: '260000' };
  assert.equal(fuelStructure(moved), fuelStructure(reply()), 'a moved amount is painted, not rendered');
  moved.consumers[0] = { ...moved.consumers[0], received: '600000' };
  assert.notEqual(fuelStructure(moved), fuelStructure(reply()), 'a request met in full draws no unmet part');
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
});
