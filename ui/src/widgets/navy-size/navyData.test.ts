import assert from 'node:assert/strict';
import test from 'node:test';
import { resolveOptions } from '../../dashboard/options.ts';
import { OPTIONS } from './definition.ts';
import { navyTotals } from './navyData.ts';

test('navyTotals selects roster/logical ships or fixed-5 strength measure', () => {
  assert.deepEqual(navyTotals(resolveOptions(OPTIONS, {})), { kind: 'navy-totals', measure: 'rosterShips', decimals: 0 });
  assert.deepEqual(navyTotals(resolveOptions(OPTIONS, { ownership: 'logical', measure: 'strength' })), { kind: 'navy-totals', measure: 'logicalStrength', decimals: 5 });
});
