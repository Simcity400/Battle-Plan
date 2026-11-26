import assert from 'node:assert/strict';
import { test } from 'node:test';
import { liveKey } from '../../charts/live.ts';
import { valuesOf } from '../../test-support/liveValues.ts';
import { NOT_LIVE_CELL } from '../../dashboard/follow/liveCells.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import type { TableModel, TableRow, TooltipExtraRow } from '../../families/table/model.ts';
import { knownValue, notApplicable, notRecorded, partialValue, zeroValue } from '../../families/value.ts';
import type { NavalRefitsReply, NavalRepairsReply, YardShip } from '../../generated/dashboard';
import { dockyardsProjection } from '../dockyards-and-repairs/dockyardsProjection.ts';
import { refitsProjection } from './refitsProjection.ts';

const ship = (handle: string, extra: Partial<YardShip> = {}): YardShip => ({
  handle, name: `Ship ${handle}`, nameRebuilt: null, design: 'Cruiser', class: 'Light cruiser', classIcon: { key: 'GFX_unit_light_cruiser_icon_medium', frame: 0 }, classIconMissing: null, hull: '60', maximum: '100', taskForce: null, policy: null, ...extra,
});

const refits = (extra: Partial<NavalRefitsReply> = {}): NavalRefitsReply => ({
  lines: [
    { handle: '7', target: 'Early Cruiser C Klasse', progress: '906000', cost: '1000000', dockyards: 1, remaining: '94000', ships: [ship('70', { hull: '100' })], shipsMissing: null },
    { handle: '8', target: null, progress: '10', cost: null, dockyards: null, remaining: null, ships: [ship('80', { maximum: null })], shipsMissing: null },
  ],
  ships: 2, dockyards: 3, costRead: 40, censusComplete: true, missing: null, warnings: [], ...extra,
});

const project = (projection: typeof refitsProjection, reply: unknown, live: Map<string, number> = new Map(), missing: ReadonlyMap<string, string> = new Map()): TableModel =>
  projection.project(createBoundedReadView({ hour: 48, panels: new Map([['main', reply]]), live: new Map([['main', valuesOf(live, key => missing.get(key) ?? NOT_LIVE_CELL)]]) }), {});

const tip = (row: TableRow | undefined): readonly TooltipExtraRow[] => (typeof row?.tooltipExtra === 'function' ? row.tooltipExtra() : row?.tooltipExtra) ?? [];
const tipValue = (row: TableRow | undefined, label: string) => tip(row).find(item => item.label === label)?.value;

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
  const shipName = known!.children![0]!.cells.name!;
  assert.equal(shipName.artShape, 'counter');
  assert.equal(shipName.art?.status, 'pending', 'a ship draws its class sprite');
  assert.equal(known!.cells.name!.artShape, undefined, 'a refit line has no class picture');
  assert.deepEqual(cell(unknown!.children![0], 'strength'), notRecorded('Maximum ship strength not recorded'));
  assert.equal(model.titleSummary, '3 dockyards');
  assert.equal(project(refitsProjection, refits({ dockyards: null })).titleSummary, null, 'no total is invented when the census is incomplete');
});

test('the refit tooltip prints the same validated cost the progress and remaining cost use; the sentinel and an unread cost are not a cost', () => {
  const tip = (cost: string | null, costRead: number | null = 40) => {
    const model = project(refitsProjection, refits({ costRead, lines: [{ ...refits().lines[0]!, cost }] }));
    return [cell(model.rows[0], 'progress'), tipValue(model.rows[0], 'Cost'), tipValue(model.rows[0], 'Remaining cost')];
  };
  assert.deepEqual(tip('1000000'), [knownValue(90.6), knownValue('10 IC'), knownValue('0.94 IC')]);
  const unknown = [notRecorded('Refit cost not recorded'), notRecorded('Refit cost not recorded'), notRecorded('Remaining cost not recorded')];
  assert.deepEqual(tip('9223372036854775807'), unknown, 'the maximum-integer sentinel');
  assert.deepEqual(tip('0'), unknown);
  assert.deepEqual(tip('1000000', null), unknown, 'a cost whose read hour is not known');
});

test('the hourly progress cell moves the share, the work done and the remaining cost together; a live cell that says missing is never covered by the held value', () => {
  const model = project(refitsProjection, refits(), new Map([[liveKey('7', 'progress'), 500000]]));
  assert.deepEqual(cell(model.rows[0], 'progress'), knownValue(50));
  assert.deepEqual([tipValue(model.rows[0], 'Work done'), tipValue(model.rows[0], 'Remaining cost')], [knownValue('5 IC'), knownValue('5 IC')]);
  const unread = 'Naval refit progress not read at this hour';
  const missing = project(refitsProjection, refits(), new Map(), new Map([[liveKey('7', 'progress'), unread], [liveKey('70', 'hull'), unread]]));
  assert.deepEqual(cell(missing.rows[0], 'progress'), notRecorded(unread));
  assert.deepEqual([tipValue(missing.rows[0], 'Work done'), tipValue(missing.rows[0], 'Remaining cost')], [notRecorded(unread), notRecorded(unread)]);
  assert.deepEqual([cell(missing.rows[0]!.children![0], 'strength'), tipValue(missing.rows[0]!.children![0], 'Strength')], [notRecorded(unread), notRecorded(unread)]);
});

test('ships linked or waiting are counted only from a whole read; the ships that were read are kept', () => {
  const unread = 'Naval refit ships read only in part at this hour (2 rows not read)';
  const line = refits().lines[0]!;
  const refit = project(refitsProjection, refits({ lines: [{ ...line, shipsMissing: unread }, { ...line, handle: '9', ships: [], shipsMissing: unread }] }));
  assert.deepEqual(tipValue(refit.rows[0], 'Ships'), partialValue('1', unread), 'the figure with the partial mark; the reason travels with it');
  assert.equal(refit.rows[0]!.children?.length, 1, 'the witnessed ship stays');
  assert.deepEqual(tipValue(refit.rows[1], 'Ships'), notRecorded(unread), 'none seen in an unread list is not zero');
  assert.deepEqual(tipValue(project(refitsProjection, refits()).rows[0], 'Ships'), knownValue('1'));
  const queueUnread = 'Naval repair queue not available at this hour';
  const port = repairs().ports[0]!;
  const docks = project(dockyardsProjection, repairs({ ports: [{ ...port, queue: [], queueMissing: queueUnread }] }));
  assert.deepEqual(tipValue(docks.rows[0], 'Ships waiting'), notRecorded(queueUnread));
  assert.equal(docks.rows[0]!.cells.name!.inlineSubtext, queueUnread, 'an unread queue does not claim no ships are waiting');
  assert.deepEqual(tipValue(project(dockyardsProjection, repairs()).rows[0], 'Ships waiting'), knownValue('1'));
});

test('refits not recorded at the hour are the body’s unavailable state with the reason', () => {
  const model = project(refitsProjection, refits({ lines: [], missing: 'Refit lines are not recorded at this hour' }));
  assert.deepEqual([model.rows.length, model.empty, model.emptyKind], [0, 'Refit lines are not recorded at this hour', 'unavailable']);
  const partial = project(refitsProjection, refits({ lines: [], censusComplete: false, dockyards: null, ships: null }));
  assert.equal(partial.emptyKind, 'unavailable', 'an incomplete census never reads as no refits');
});

const repairs = (extra: Partial<NavalRepairsReply> = {}): NavalRepairsReply => ({
  ports: [
    { province: 11, name: 'Brest', priority: 2, policy: 'Normal', dockyards: 0, queue: [ship('90', { hull: '0' })], queueMissing: null },
    { province: 12, name: null, priority: null, policy: null, dockyards: null, queue: [], queueMissing: null },
  ],
  usedDockyards: 0, repairLimit: 29, queuedShips: 1, censusComplete: true, warnings: [], ...extra,
});

test('a port shows its priority and dockyards, its queued ships their strength, and the title the product’s totals', () => {
  const model = project(dockyardsProjection, repairs());
  const [brest, unnamed] = model.rows;
  assert.deepEqual([cell(brest, 'priority'), cell(brest, 'dockyards')], [knownValue(2), zeroValue(0)]);
  assert.deepEqual(cell(brest!.children![0], 'strength'), zeroValue(0));
  assert.deepEqual(cell(unnamed, 'name'), notRecorded('Port name unavailable'));
  assert.equal(brest!.cells.name!.inlineSubtext, undefined, 'a port with ships waiting needs no empty-queue label');
  assert.equal(unnamed!.cells.name!.inlineSubtext, 'No ships waiting', 'a priority-only port with no queued ships says so');
  assert.equal(cell(unnamed, 'dockyards')?.kind, 'notRecorded');
  assert.equal(unnamed!.children, undefined, 'a port with no queue has nothing to open');
  assert.equal(model.titleSummary, '1 ship waiting · 0 / 29 repair dockyards in use');
  assert.equal(project(dockyardsProjection, repairs({ queuedShips: null, repairLimit: null })).titleSummary, null);
});
