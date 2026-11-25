import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { buildRowSourceFromHead } from '../../families/table/rowSource.ts';
import type { LandBattlesTailReply } from '../../generated/dashboard';
import { landBattlesProjection } from './landBattlesProjection.ts';

test('paired figures keep the recorded side when the other side or its IC cost is missing', () => {
  const head: LandBattlesTailReply = { branchId: '1', total: '1', offset: '0', missing: [], warnings: [], nextChange: null, rows: [{
    key: '1:3:10', hour: 10, exact: false, province: null, state: null, countries: [[], []], countriesUnknown: [true, true],
    countriesHour: [10, 10], casualties: [0, null], icLost: [null, 12], divisions: [2, 4], result: 'unknown',
  }] };
  const source = buildRowSourceFromHead('branch', head, undefined, 18)!;
  const view = createBoundedReadView({ hour: 18, rows: new Map([['main', source]]) });
  const row = landBattlesProjection.project(view, { byDate: true }).rows[0]!;
  assert.deepEqual(row.cells.casualties!.pair!.map(x => x.kind), ['zero', 'notRecorded']);
  assert.equal(row.cells.casualties!.value.kind, 'notRecorded');
  assert.deepEqual(row.cells.icLost!.splitBar, [{ fraction: 1, fill: '', hatched: true }]);
  assert.deepEqual(row.cells.divisions!.value, { kind: 'known', value: 6 });
  assert.equal(row.cells.status!.value.kind, 'notRecorded');
  assert.equal(row.cells.date!.boundedDate, 'by');
  assert.equal(row.cells.action!.actionLabel, 'Watch');
});
