import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { buildColumnLayouts, layoutFamilyTable, measureFixedColumnWidths } from '../../families/table/layout.ts';
import { resolveTableRows } from '../../families/table/model.ts';
import { buildRowSourceFromHead } from '../../families/table/rowSource.ts';
import { tableDefaults } from '../../families/table/settings.ts';
import { LOG_TABLE_BASE } from '../../families/table/logHelpers.ts';
import type { DivisionLogTailReply } from '../../generated/dashboard';
import { SERVICE_COLUMNS } from '../division-service-record/definition.ts';
import { DIVISION_LOG_COLUMNS } from './definition.ts';
import { divisionLogProjection } from './divisionLogProjection.ts';

test('expanded groups retain their paged children and resolve identities at the supplying hours', () => {
  const group = { key: 'group:92:92:1', hour: 92, observed: 92, slot: 1, tag: 'ABC', parent: null, event: null, members: 2, open: true };
  const child = { ...group, key: '17', parent: group.key, members: 0, open: false, event: {
    division: '17', kind: 0, status: 'first_observed', exact: false, afterHour: 87, name: null, nameMissing: 'Division name unavailable',
    icon: null, iconMissing: null, template: 2, templateName: 'Engineers', templateMissing: null, province: 12, placeHour: 89,
    killerSlot: null, killerTag: null, killerDivision: null, matchedDeaths: null,
  } };
  const head: DivisionLogTailReply = {
    branchId: '1', available: true, unavailable: null, total: '3', offset: '0', counts: [{ key: 'created', total: '2' }], censusIncomplete: false,
    templates: [{ id: 2, firstHour: 91, name: 'Engineers' }, { id: 3, firstHour: 102, name: 'Reserves' }],
    rows: [group, child, { ...child, key: '18', event: { ...child.event, division: '18', kind: 1, status: 'division_destroyed' } }], missing: [], nextChange: 102, warnings: [],
  };
  const source = buildRowSourceFromHead('branch', head, undefined, 100)!;
  const view = createBoundedReadView({ hour: 100, rows: new Map([['main', source]]), identity: {
    resolve: (kind, key, atHour) => ({ kind, key, atHour, name: { status: 'ready', value: `${kind} ${key} at ${atHour}` },
      holder: { status: 'absent', reason: 'fixture' }, validInterval: [atHour, atHour], nextChangeAt: Infinity }),
  } });
  const model = divisionLogProjection.project(view, { ranged: false, events: 'both', prefix: 'branch-', selected: [] });
  assert.equal(model.endNote, 'Start of the recording');
  assert.equal(divisionLogProjection.project(view, { ranged: true, events: 'both', prefix: 'branch-', selected: [] }).endNote, undefined);
  const flat = resolveTableRows(model, tableDefaults(DIVISION_LOG_COLUMNS, LOG_TABLE_BASE));
  assert.deepEqual(flat.flatMap(row => row.kind === 'data' ? [[row.key, row.level, row.parent, row.isOpen]] : []),
    [[group.key, 1, null, true], ['17', 2, group.key, false], ['18', 2, group.key, false]]);
  assert.deepEqual(model.kindFilter?.items.map(item => item.label), ['Engineers']);
  assert.equal(model.rows[0]!.cells.item!.value.kind, 'known');
  assert.deepEqual(model.rows[1]!.cells.place!.value, { kind: 'known', value: 'province 12 at 89' });
  assert.equal(model.rows[1]!.cells.item!.value.kind, 'notRecorded');
  const extra = model.rows[1]!.tooltipExtra;
  const tooltip = typeof extra === 'function' ? extra() : extra;
  assert.deepEqual(tooltip?.find(line => line.label === 'Country')?.value, { kind: 'known', value: 'country ABC at 92' });
  const lost = model.rows[2]!;
  const lostExtra = typeof lost.tooltipExtra === 'function' ? lost.tooltipExtra() : lost.tooltipExtra;
  assert.equal(lost.cells.place!.tooltipLabel, 'Last seen at', 'a removal the recording only bounds names its place as where the division was last seen');
  assert.equal(lostExtra?.some(line => line.label === 'Last seen at' || line.label === 'Place'), false, 'and states it once');
});

test('the minimum division log keeps both country flags visible without scrolling, and flags-only and named country columns size apart', () => {
  const settings = tableDefaults(DIVISION_LOG_COLUMNS, LOG_TABLE_BASE);
  const serviceDefaults = tableDefaults(SERVICE_COLUMNS, LOG_TABLE_BASE);
  const serviceNames = { ...serviceDefaults, columns: { ...serviceDefaults.columns, country: { ...serviceDefaults.columns.country!, showNames: true } } };
  const measured = { date: 60, kind: 36, item: 94, country: 150, place: 140, outcome: 120, action: 20 };
  const columns = buildColumnLayouts(DIVISION_LOG_COLUMNS, settings, measured);
  const wide = layoutFamilyTable(columns, 960);
  assert.ok(wide.columns.some(column => column.key === 'place'));
  const narrow = layoutFamilyTable(columns, 320);
  assert.equal(narrow.scrollable, false);
  assert.deepEqual(narrow.columns.find(column => column.key === 'item')?.inlineFlags, ['country', 'outcome']);
  assert.deepEqual(narrow.columns.filter(column => column.display === 'flag').map(column => column.key), ['country', 'outcome']);
  assert.ok(narrow.columns.every(column => column.key !== 'place'));

  const theme = { font: 'sans-serif', fontSize: { small: 11, body: 12 } } as never;
  const measure = (text: string) => text.length * 7;
  const flagsOnly = { ...settings, columns: { ...settings.columns, country: { ...settings.columns.country!, showNames: false } } };
  const fixed = measureFixedColumnWidths(buildColumnLayouts(DIVISION_LOG_COLUMNS, flagsOnly), flagsOnly, theme, measure, undefined, { hideHeader: true });
  assert.equal(fixed.country, 48, 'with names off the country column is one flag wide');
  const named = measureFixedColumnWidths(buildColumnLayouts(SERVICE_COLUMNS, serviceNames), serviceNames, theme, measure, { country: ['German Reich · Kingdom of Hungary'] }, { hideHeader: true });
  assert.ok(named.country! > 68, `with names on the service record country column takes the names (${named.country})`);
});
