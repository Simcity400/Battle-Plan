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
    facts: { level: null, levelName: null, experienceRaw: null, casualtiesTaken: null, medals: 0, awards: [], historyMissing: null,
      enemyKilled: null, enemyKilledMissing: 'Enemy killed is not recorded at this hour: added in format 152' },
    medals: [], total: '1', offset: '0', missing: [], unavailable: null, warnings: [], nextChange: null,
    rows: [{ key: 'loss:17', hour: 92, observed: 94, kind: 'removed', exact: false, tag: 'ABC', other: 'DEF',
      province: 5, placeHour: 91, title: null, status: 'division_destroyed', medal: null }],
  };
  const viewOf = (reply: ServiceRecordTailReply) => createBoundedReadView({ hour: 100, rows: new Map([['main', buildRowSourceFromHead('branch', reply, undefined, 100)!]]), identity: {
    resolve: (kind, key, atHour) => ({ kind, key, atHour, name: { status: 'ready', value: kind === 'battle' ? 'Second Battle of Shale Peak' : `${key} at ${atHour}` },
      holder: { status: 'absent', reason: 'fixture' }, validInterval: [atHour, atHour], nextChangeAt: Infinity }),
  } });
  const model = serviceRecordProjection.project(viewOf(head), { campaign: 'c', chosen: subjectToken('c', '17') });
  assert.equal(model.empty, undefined);
  assert.deepEqual(model.subjectSummary?.country, { kind: 'known', value: 'ABC at 91' });
  assert.deepEqual(model.subjectSummary?.figures.map(figure => [figure.label, figure.value.kind]), [['Casualties taken', 'notRecorded'], ['Enemy killed', 'notRecorded']]);
  const killed = model.subjectSummary?.figures[1];
  assert.deepEqual(killed?.value, { kind: 'notRecorded', reason: 'Enemy killed is not recorded at this hour: added in format 152' }, 'a recording without the count says why');
  assert.match(killed?.hint ?? '', /shared among the divisions that damaged it/, 'the tooltip says how the figure is worked out');
  const counted = serviceRecordProjection.project(viewOf({ ...head, facts: { ...head.facts!, enemyKilled: 480, enemyKilledMissing: null } }),
    { campaign: 'c', chosen: subjectToken('c', '17') });
  assert.deepEqual(counted.subjectSummary?.figures[1]?.value, { kind: 'known', value: 480 });
  assert.equal(model.kindFilter?.single?.label, 'Second division');
  const cells = model.rows[0]!.cells;
  assert.deepEqual(cells.place!.value, { kind: 'known', value: '5 at 91' });
  assert.deepEqual(cells.country!.flags?.map(flag => [flag.tag, Boolean(flag.dimmed)]), [['DEF', false], ['ABC', true]]);
  const pendingBattle: ServiceRecordTailReply = {
    ...head,
    rows: [{ key: 'battle:17:90', hour: 90, observed: 90, kind: 'battle', exact: true, tag: 'ABC', other: 'DEF',
      province: 5, placeHour: 90, title: null, status: 'won', medal: null }],
  };
  const pendingModel = serviceRecordProjection.project(
    createBoundedReadView({ hour: 100, rows: new Map([['main', buildRowSourceFromHead('branch', pendingBattle, undefined, 100)!]]) }),
    { campaign: 'c', chosen: subjectToken('c', '17') },
  );
  assert.deepEqual(pendingModel.rows[0]!.cells.kind!.value, { kind: 'known', value: 'Land battle' });
  assert.deepEqual(pendingModel.rows[0]!.cells.item!.value, { kind: 'pending', reason: 'Name is loading' });
  const named = serviceRecordProjection.project(viewOf(pendingBattle), { campaign: 'c', chosen: subjectToken('c', '17') });
  assert.deepEqual(named.rows[0]!.cells.item!.value, { kind: 'known', value: 'Second Battle of Shale Peak' });
});
