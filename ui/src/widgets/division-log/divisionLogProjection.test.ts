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

test('expanded groups retain their paged children and carry place names without another request', () => {
  const group = { key: 'group:92:92:1', hour: 92, observed: 92, slot: 1, tag: 'ABC', parent: null, event: null, members: 3, open: true };
  const event = {
    division: '17', kind: 0, status: 'first_observed', exact: false, afterHour: 87, name: null, nameMissing: 'Division name unavailable',
    icon: null, iconMissing: null, template: 2, templateName: 'Engineers', templateMissing: null, province: 12, placeHour: 89,
    killerSlot: null, killerTag: null, killerDivision: null, matchedDeaths: null,
  };
  const child = { ...group, key: '17', parent: group.key, members: 0, open: false, event, placeName: 'Province at the supplying hour' };
  const missing = { ...group, key: '18', parent: group.key, members: 0, open: false, event: { ...event, division: '18', kind: 1, status: 'division_destroyed', name: '', nameMissing: null }, placeMissing: 'Part of the place history is unavailable' };
  const blank = { ...missing, key: '19', placeName: '' };
  const head: DivisionLogTailReply = {
    branchId: '1', available: true, unavailable: null, total: '4', offset: '0', counts: [{ key: 'created', total: '2' }], censusIncomplete: false,
    templates: [{ id: 2, firstHour: 91, name: 'Engineers' }, { id: 3, firstHour: 102, name: 'Reserves' }],
    rows: [group, child, missing, blank], missing: [], nextChange: 102, warnings: [],
  };
  const source = buildRowSourceFromHead('branch', head, undefined, 100)!;
  let placeReads = 0;
  const view = createBoundedReadView({ hour: 100, rows: new Map([['main', source]]), identity: {
    resolve: (kind, key, atHour) => {
      if (kind === 'province') placeReads++;
      return { kind, key, atHour, name: { status: 'ready', value: `${kind} ${key} at ${atHour}` },
        holder: { status: 'absent', reason: 'fixture' }, validInterval: [atHour, atHour], nextChangeAt: Infinity };
    },
  } });
  const model = divisionLogProjection.project(view, { ranged: false, events: 'both', prefix: 'branch-', selected: [] });
  assert.equal(model.endNote, 'Start of the recording');
  assert.equal(divisionLogProjection.project(view, { ranged: true, events: 'both', prefix: 'branch-', selected: [] }).endNote, undefined);
  const flat = resolveTableRows(model, tableDefaults(DIVISION_LOG_COLUMNS, LOG_TABLE_BASE));
  assert.deepEqual(flat.flatMap(row => row.kind === 'data' ? [[row.key, row.level, row.parent, row.isOpen]] : []),
    [[group.key, 1, null, true], ['17', 2, group.key, false], ['18', 2, group.key, false], ['19', 2, group.key, false]]);
  assert.deepEqual(model.kindFilter?.items.map(item => item.label), ['Engineers']);
  assert.equal(model.rows[0]!.cells.item!.value.kind, 'known');
  assert.deepEqual(model.rows[1]!.cells.place!.value, { kind: 'known', value: 'Province at the supplying hour' });
  assert.deepEqual(model.rows[1]!.cells.item!.value, { kind: 'notRecorded', reason: 'Division name unavailable' });
  const extra = model.rows[1]!.tooltipExtra;
  const tooltip = typeof extra === 'function' ? extra() : extra;
  assert.deepEqual(tooltip?.find(line => line.label === 'Country')?.value, { kind: 'known', value: 'country ABC at 92' });
  const lost = model.rows[2]!;
  assert.equal(lost.name, 'Division name not recorded');
  assert.deepEqual(lost.cells.item!.value, { kind: 'notRecorded', reason: 'Division name not recorded for this event' });
  const lostExtra = typeof lost.tooltipExtra === 'function' ? lost.tooltipExtra() : lost.tooltipExtra;
  assert.equal(lost.cells.place!.tooltipLabel, 'Last seen at', 'a removal the recording only bounds names its place as where the division was last seen');
  assert.deepEqual(lost.cells.place!.value, { kind: 'notRecorded', reason: 'Part of the place history is unavailable' });
  assert.deepEqual(model.rows[3]!.cells.place!.value, { kind: 'notRecorded', reason: 'Name not recorded' });
  assert.equal(placeReads, 0);
  assert.equal(lostExtra?.some(line => line.label === 'Last seen at' || line.label === 'Place'), false, 'and states it once');
});

test('the minimum division log keeps both country flags visible without scrolling, and flags-only and named country columns size apart', async () => {
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
  const pair = 'German Reich · Kingdom of Hungary';
  const named = measureFixedColumnWidths(buildColumnLayouts(SERVICE_COLUMNS, serviceNames), serviceNames, theme, measure, { country: [pair] }, { hideHeader: true, flags: { country: 2 } });
  assert.ok(named.country! >= measure(pair) + 2 * (20 + 6) + 2 + 20, `with names on, a two-country service row fits both flags and both names (${named.country})`);

  const { tableLane } = await import('../../families/table/laneView.ts');
  const { readyArt } = await import('../../families/art/types.ts');
  const { knownValue } = await import('../../families/value.ts');
  const narrowModel = {
    columns: DIVISION_LOG_COLUMNS,
    hour: 100,
    hideHeader: true,
    unsorted: true,
    rows: [{
      key: 'div-1',
      name: '1st Infantry',
      cells: {
        date: { value: knownValue(100) },
        kind: { value: knownValue('Lost'), glyph: 'loss' as const },
        item: { value: knownValue('1st Infantry') },
        country: { value: knownValue('Germany'), flags: [{ tag: 'GER', art: readyArt('flag://div-owner') }] },
        place: { value: knownValue('Berlin') },
        outcome: { value: knownValue('Destroyed by SOV'), flags: [{ tag: 'SOV', art: readyArt('flag://div-killer') }] },
        action: { value: knownValue('Inspect') },
      },
    }],
  };
  const canvasCtx = { font: '', measureText: (text: string) => ({ width: text.length * 7 }) } as unknown as OffscreenCanvasRenderingContext2D;
  const surface = { width: 320, height: 110, dpr: 1 };
  const binding = { family: 'table', settings, width: 320, height: 110, scrollLeft: 0, scrollTop: 0, options: {} } as never;
  const layout = tableLane.layout(narrowModel, binding, surface as never, theme, canvasCtx, 100, { previous: null, now: 1000, speed: 24 } as never);
  const visibleUrls = [...tableLane.visibleArt({ model: narrowModel, layout, binding, surface, theme, hour: 100, now: 1000, changedAt: 1000, continuity: 1, speed: 24, before: null } as never)];
  assert.ok(visibleUrls.includes('flag://div-owner'), `320 px Division Log keeps inline owner flag in visibleArt (got ${JSON.stringify(visibleUrls)})`);
  assert.ok(visibleUrls.includes('flag://div-killer'), `320 px Division Log keeps inline killer flag in visibleArt (got ${JSON.stringify(visibleUrls)})`);

  const serviceModel = {
    columns: SERVICE_COLUMNS,
    hour: 100,
    hideHeader: true,
    unsorted: true,
    rows: [{
      key: 'svc-1',
      name: '1st Infantry',
      cells: {
        date: { value: knownValue(100) },
        kind: { value: knownValue('Transfer'), glyph: 'transfer' as const },
        item: { value: knownValue('1st Infantry') },
        country: {
          value: knownValue('Germany · Hungary'),
          flags: [
            { tag: 'GER', art: readyArt('flag://svc-ger') },
            { tag: 'HUN', art: readyArt('flag://svc-hun') },
          ],
        },
        place: { value: knownValue('Vienna') },
      },
    }],
  };
  const serviceBinding = { family: 'table', settings: serviceDefaults, width: 320, height: 110, scrollLeft: 0, scrollTop: 0, options: {} } as never;
  const serviceLayout = tableLane.layout(serviceModel, serviceBinding, surface as never, theme, canvasCtx, 100, { previous: null, now: 1000, speed: 24 } as never);
  const serviceUrls = [...tableLane.visibleArt({ model: serviceModel, layout: serviceLayout, binding: serviceBinding, surface, theme, hour: 100, now: 1000, changedAt: 1000, continuity: 2, speed: 24, before: null } as never)];
  assert.deepEqual(serviceUrls, ['flag://svc-ger'], '320 px Division Service Record pins only the drawn first flag and omits the clipped second flag');

  const widePlaceSettings = {
    ...settings,
    columns: { ...settings.columns, place: { ...settings.columns.place!, width: 180 } },
  };
  const glyphModel = {
    ...narrowModel,
    rows: [{
      ...narrowModel.rows[0]!,
      cells: {
        ...narrowModel.rows[0]!.cells,
        outcome: { value: knownValue('Destroyed by SOV'), glyph: 'loss' as const, flags: [{ tag: 'SOV', art: readyArt('flag://div-killer-post-glyph') }] },
      },
    }],
  };
  const scrolledBinding = { family: 'table', settings: widePlaceSettings, width: 320, height: 110, scrollLeft: 210, scrollTop: 0, options: {} } as never;
  const scrolledLayout = tableLane.layout(glyphModel, scrolledBinding, surface as never, theme, canvasCtx, 100, { previous: null, now: 1000, speed: 24 } as never);
  const scrolledUrls = [...tableLane.visibleArt({ model: glyphModel, layout: scrolledLayout, binding: scrolledBinding, surface, theme, hour: 100, now: 1000, changedAt: 1000, continuity: 3, speed: 24, before: null } as never)];
  assert.ok(scrolledUrls.includes('flag://div-killer-post-glyph'), `post-glyph killer flag exposed past pinned boundary stays in visibleArt (got ${JSON.stringify(scrolledUrls)})`);

  const wideDateSettings = {
    ...settings,
    columns: { ...settings.columns, date: { ...settings.columns.date!, width: 220 } },
  };
  const clippedInlineBinding = { family: 'table', settings: wideDateSettings, width: 320, height: 110, scrollLeft: 0, scrollTop: 0, options: {} } as never;
  const clippedInlineLayout = tableLane.layout(narrowModel, clippedInlineBinding, surface as never, theme, canvasCtx, 100, { previous: null, now: 1000, speed: 24 } as never);
  const clippedInlineUrls = [...tableLane.visibleArt({ model: narrowModel, layout: clippedInlineLayout, binding: clippedInlineBinding, surface, theme, hour: 100, now: 1000, changedAt: 1000, continuity: 4, speed: 24, before: null } as never)];
  assert.deepEqual(clippedInlineUrls, ['flag://div-owner'], `narrow Division Log with 220 px Date column omits killer flag past right edge (got ${JSON.stringify(clippedInlineUrls)})`);

  const narrowRemovalSettings = {
    ...settings,
    columns: {
      ...settings.columns,
      place: { ...settings.columns.place!, width: 180 },
      outcome: { ...settings.columns.outcome!, width: 40 },
      action: { ...settings.columns.action!, width: 140 },
    },
  };
  const spillBinding = { family: 'table', settings: narrowRemovalSettings, width: 320, height: 110, scrollLeft: 220, scrollTop: 0, options: {} } as never;
  const spillLayout = tableLane.layout(glyphModel, spillBinding, surface as never, theme, canvasCtx, 100, { previous: null, now: 1000, speed: 24 } as never);
  const spillUrls = [...tableLane.visibleArt({ model: glyphModel, layout: spillLayout, binding: spillBinding, surface, theme, hour: 100, now: 1000, changedAt: 1000, continuity: 5, speed: 24, before: null } as never)];
  assert.ok(spillUrls.includes('flag://div-killer-post-glyph'), `post-glyph killer flag extending past a 40 px Removal column stays in visibleArt (got ${JSON.stringify(spillUrls)})`);

  const { notApplicable } = await import('../../families/value.ts');
  const naModel = {
    ...glyphModel,
    rows: [{
      ...glyphModel.rows[0]!,
      cells: {
        ...glyphModel.rows[0]!.cells,
        item: { value: notApplicable('Not applicable') },
      },
    }],
  };
  const naDateSettings = {
    ...settings,
    columns: { ...settings.columns, date: { ...settings.columns.date!, width: 209 } },
  };
  const naBinding = { family: 'table', settings: naDateSettings, width: 320, height: 110, scrollLeft: 0, scrollTop: 0, options: {} } as never;
  const naLayout = tableLane.layout(naModel, naBinding, surface as never, theme, canvasCtx, 100, { previous: null, now: 1000, speed: 24 } as never);
  const naUrls = [...tableLane.visibleArt({ model: naModel, layout: naLayout, binding: naBinding, surface, theme, hour: 100, now: 1000, changedAt: 1000, continuity: 6, speed: 24, before: null } as never)];
  assert.deepEqual(naUrls, ['flag://div-owner'], `notApplicable primary cell uses shared missingMarkWidth and omits killer flag at x=320 (got ${JSON.stringify(naUrls)})`);
});
