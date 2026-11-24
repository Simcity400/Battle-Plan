import assert from 'node:assert/strict';
import { test } from 'node:test';
import { liveKey, valuesOf } from '../../charts/live.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import type { TableModel, TableRow } from '../../families/table/model.ts';
import { knownValue, notApplicable, notRecorded, zeroValue } from '../../families/value.ts';
import type { NavalRefitsReply, NavalRepairsReply, YardShip } from '../../generated/dashboard';
import { dockyardsProjection } from '../dockyards-and-repairs/dockyardsProjection.ts';
import { refitsProjection } from './refitsProjection.ts';

const ship = (handle: string, extra: Partial<YardShip> = {}): YardShip => ({
  handle, name: `Ship ${handle}`, nameRebuilt: null, design: 'Cruiser', class: 'Light cruiser', hull: '60', maximum: '100', taskForce: null, policy: null, ...extra,
});

const refits = (extra: Partial<NavalRefitsReply> = {}): NavalRefitsReply => ({
  lines: [
    { handle: '7', target: 'Early Cruiser C Klasse', progress: '906000', cost: '1000000', dockyards: 1, remaining: '94000', ships: [ship('70', { hull: '100' })] },
    { handle: '8', target: null, progress: '10', cost: null, dockyards: null, remaining: null, ships: [ship('80', { maximum: null })] },
  ],
  ships: 2, dockyards: 3, costRead: 40, censusComplete: true, missing: null, warnings: [], ...extra,
});

const project = (projection: typeof refitsProjection, reply: unknown, live: Map<string, number> = new Map()): TableModel =>
  projection.project(createBoundedReadView({ hour: 48, panels: new Map([['main', reply]]), live: new Map([['main', valuesOf(live, 'Not in the answer')]]) }), {});

const cell = (row: TableRow | undefined, column: string) => row?.cells[column]?.value;

test('a refit line shows the product’s progress against its cost, its dockyards and the dockyards total; a line without a cost says so', () => {
  const model = project(refitsProjection, refits());
  const [known, unknown] = model.rows;
  assert.deepEqual(cell(known, 'progress'), knownValue(90.6));
  assert.deepEqual(cell(known, 'dockyards'), knownValue(1));
  assert.deepEqual(cell(known, 'strength'), notApplicable('Shown on each ship'));
  assert.deepEqual(cell(unknown, 'progress'), notRecorded('Refit cost not recorded'));
  assert.equal(cell(unknown, 'dockyards')?.kind, 'notRecorded', 'dockyards the line metadata does not hold are missing, not zero');
  assert.deepEqual(cell(known!.children![0], 'strength'), knownValue(100));
  assert.deepEqual(cell(unknown!.children![0], 'strength'), notRecorded('Maximum ship strength not recorded'));
  assert.equal(model.titleSummary, '3 dockyards');
  assert.equal(project(refitsProjection, refits({ dockyards: null })).titleSummary, null, 'no total is invented when the census is incomplete');
});

test('the hourly progress cell moves the share without a new reply', () => {
  const model = project(refitsProjection, refits(), new Map([[liveKey('7', 'progress'), 500000]]));
  assert.deepEqual(cell(model.rows[0], 'progress'), knownValue(50));
});

test('refits not recorded at the hour are the body’s unavailable state with the reason', () => {
  const model = project(refitsProjection, refits({ lines: [], missing: 'Refit lines are not recorded at this hour' }));
  assert.deepEqual([model.rows.length, model.empty, model.emptyKind], [0, 'Refit lines are not recorded at this hour', 'unavailable']);
  const partial = project(refitsProjection, refits({ lines: [], censusComplete: false, dockyards: null, ships: null }));
  assert.equal(partial.emptyKind, 'unavailable', 'an incomplete census never reads as no refits');
});

const repairs = (extra: Partial<NavalRepairsReply> = {}): NavalRepairsReply => ({
  ports: [
    { province: 11, name: 'Brest', priority: 2, policy: 'Normal', dockyards: 0, queue: [ship('90', { hull: '0' })] },
    { province: 12, name: null, priority: null, policy: null, dockyards: null, queue: [] },
  ],
  usedDockyards: 0, repairLimit: 29, queuedShips: 1, censusComplete: true, warnings: [], ...extra,
});

test('a port shows its priority and dockyards, its queued ships their strength, and the title the product’s totals', () => {
  const model = project(dockyardsProjection, repairs());
  const [brest, unnamed] = model.rows;
  assert.deepEqual([cell(brest, 'priority'), cell(brest, 'dockyards')], [knownValue(2), zeroValue(0)]);
  assert.deepEqual(cell(brest!.children![0], 'strength'), zeroValue(0));
  assert.deepEqual(cell(unnamed, 'name'), notRecorded('Port name unavailable'));
  assert.equal(cell(unnamed, 'dockyards')?.kind, 'notRecorded');
  assert.equal(unnamed!.children, undefined, 'a port with no queue has nothing to open');
  assert.equal(model.titleSummary, '1 ship waiting · 0 / 29 repair dockyards in use');
  assert.equal(project(dockyardsProjection, repairs({ queuedShips: null, repairLimit: null })).titleSummary, null);
});
