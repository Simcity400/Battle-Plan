import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { buildRowSourceFromHead } from '../../families/table/rowSource.ts';
import type { ShipsSunkTailReply } from '../../generated/dashboard';
import { shipsSunkProjection } from './shipsSunkProjection.ts';

test('ship rows name the class and keep the victim dimmed, with the missing mark for an unrecorded cause', () => {
  const head: ShipsSunkTailReply = {
    branchId: '1', total: '1', offset: '0', rows: [{
      key: '0', hour: 10, observed: 20, name: 'Meridian', classToken: 17, recordedClass: 'Corvette', attacker: 'AAA', victim: 'BBB',
      seaZone: null, battle: { kind: 'notRecorded', reason: 'No recorded link' }, battleProvince: null,
      sinkingCause: { kind: 'notRecorded', reason: 'Cause not recorded' },
    }],
    classes: [{ firstHour: 20, class: { token: 17, name: null, key: 'mod_corvette', icon: null } }],
    nextChange: null, missing: [], warnings: [],
  };
  const source = buildRowSourceFromHead('branch', head, undefined, 20)!;
  const view = createBoundedReadView({ hour: 20, rows: new Map([['main', source]]) });
  const model = shipsSunkProjection.project(view, { prefix: 'branch:', selected: [] });
  const row = model.rows[0]!;
  assert.equal(model.empty, undefined);
  assert.equal(model.titleSummary, '1 sunk');
  assert.equal(model.kindFilter?.allLabel, 'All classes');
  assert.equal(row.meta, 10);
  assert.equal(row.cells.item?.inlineSubtext, 'Corvette');
  assert.deepEqual(row.cells.country?.flags?.map(flag => [flag.tag, !!flag.dimmed]), [['AAA', false], ['BBB', true]]);
  assert.equal(row.cells.cause?.value.kind, 'notRecorded');
  assert.equal(row.cells.cause?.missingFigure, 'Cause');
});
