import assert from 'node:assert/strict';
import test from 'node:test';
import { HourStore, planRequests } from '../../dashboard/countryFacts.ts';
import { portraitSpec, PORTRAITS_PER_REQUEST } from './portraits.ts';

test('a roster of portraits is asked a few at a time, the rows shown first leading', () => {
  const spec = portraitSpec(86);
  const wanted = Array.from({ length: 1042 }, (_, row) => ({ hour: row < 1000 ? 100 : 200 + row, tag: String(9000 - row) }));
  const plan = planRequests(new HourStore<unknown>(64), wanted, 64, spec.perRequest, spec.requestOrder);
  assert.ok(plan.every(([, references]) => references.length <= PORTRAITS_PER_REQUEST), 'no request names a whole roster');
  assert.deepEqual(plan[0], [100, Array.from({ length: PORTRAITS_PER_REQUEST }, (_, index) => String(9000 - index))], 'the first rows of the list are the first asked');
  assert.equal(plan.reduce((sum, [, references]) => sum + references.length, 0), 1042);
});
