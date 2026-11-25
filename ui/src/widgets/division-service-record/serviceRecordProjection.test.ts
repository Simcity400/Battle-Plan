import assert from 'node:assert/strict';
import test from 'node:test';
import { subjectToken } from '../../dashboard/chosen.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { buildRowSourceFromHead } from '../../families/table/rowSource.ts';
import type { ServiceRecordTailReply } from '../../generated/dashboard';
import { serviceRecordProjection } from './serviceRecordProjection.ts';

test('a removed division keeps its last recorded facts and the opponent of its removal', () => {
  const head: ServiceRecordTailReply = {
    branchId: '1', selected: { division: '17', name: 'Second division', template: 'Infantry',
      sprite: null, lastSeen: 91, removedAt: 92, removed: 'division_destroyed', tag: 'ABC', medals: 0 },
    facts: { level: null, levelName: null, experienceRaw: null, casualtiesTaken: null, medals: 0, awards: [], historyMissing: null },
    medals: [], total: '1', offset: '0', missing: [], unavailable: null, warnings: [], nextChange: null,
    rows: [{ key: 'loss:17', hour: 92, observed: 94, kind: 'removed', exact: false, tag: 'ABC', other: 'DEF',
      province: 5, placeHour: 91, title: null, status: 'division_destroyed', medal: null }],
  };
  const source = buildRowSourceFromHead('branch', head, undefined, 100)!;
  const view = createBoundedReadView({ hour: 100, rows: new Map([['main', source]]), identity: {
    resolve: (kind, key, atHour) => ({ kind, key, atHour, name: { status: 'ready', value: `${key} at ${atHour}` },
      holder: { status: 'absent', reason: 'fixture' }, validInterval: [atHour, atHour], nextChangeAt: Infinity }),
  } });
  const model = serviceRecordProjection.project(view, { campaign: 'c', chosen: subjectToken('c', '17') });
  assert.equal(model.empty, undefined);
  assert.equal(model.subjectSummary?.country, 'ABC at 91');
  assert.deepEqual(model.subjectSummary?.metrics.map(metric => metric.value), ['—', '—']);
  assert.equal(model.kindFilter?.single?.label, 'Second division');
  const cells = model.rows[0]!.cells;
  assert.deepEqual(cells.place!.value, { kind: 'known', value: '5 at 91' });
  assert.deepEqual(cells.country!.flags?.map(flag => [flag.tag, Boolean(flag.dimmed)]), [['DEF', false], ['ABC', true]]);
  assert.equal(cells.date!.boundedDate, 'by');
});
