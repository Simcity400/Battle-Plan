import assert from 'node:assert/strict';
import { test } from 'node:test';
import { liveKey, valuesOf } from '../../charts/live.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { knownValue, notRecorded, zeroValue } from '../../families/value.ts';
import type { AirMissionsReply } from '../../generated/dashboard';
import { airMissionsProjection } from './airMissionsProjection.ts';

const reply: AirMissionsReply = {
  readHour: 6, missing: null, theatresMissing: null, warnings: [], theatres: null,
  missions: [{ id: '0', name: 'Air Superiority', icon: null }, { id: '4', name: 'Air Recon', icon: null }],
  regions: [{ region: '10', name: 'Eastern Poland', missing: null, missions: ['0', '4'] }, { region: '11', name: null, missing: 'Region 11 has no localised name', missions: ['4'] }],
};

const project = (raw: unknown, live: Map<string, number>) =>
  airMissionsProjection.project(createBoundedReadView({ hour: 8, panels: new Map([['main', raw]]), live: new Map([['main', valuesOf(live, 'Not in the answer')]]) }), {});

test('one column per mission the recording holds, in its order; a mission no wing of the region flies is zero; an unread cell is missing', () => {
  const model = project(reply, new Map([[liveKey('10', 'm0'), 2197], [liveKey('10', 'wings'), 45], [liveKey('11', 'wings'), 2]]));
  assert.deepEqual(model.columns.map(column => [column.key, column.label]), [['region', 'Region'], ['m0', 'Air Superiority'], ['m4', 'Air Recon'], ['wings', 'Wings']]);
  const [poland, other] = model.rows;
  assert.deepEqual([poland!.cells.m0!.value, poland!.cells.wings!.value], [knownValue(2197), knownValue(45)]);
  assert.deepEqual(poland!.cells.m4!.value, notRecorded('Not in the answer'));
  assert.deepEqual(other!.cells.m0!.value, zeroValue(0));
  assert.deepEqual(other!.cells.region!.value, notRecorded('Region 11 has no localised name'));
});

test('missions not recorded at the hour are the body’s unavailable state', () => {
  const model = project({ ...reply, regions: [], missions: [], missing: 'Air missions are not recorded at this hour' }, new Map());
  assert.deepEqual([model.rows.length, model.emptyKind, model.empty], [0, 'unavailable', 'Air missions are not recorded at this hour']);
});
