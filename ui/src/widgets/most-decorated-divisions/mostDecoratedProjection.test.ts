import assert from 'node:assert/strict';
import { test } from 'node:test';
import { resolveOptions, type OptionValue } from '../../dashboard/options.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import type { DeclaredCountryFilter, FamilyPlanContext } from '../../families/plan.ts';
import type { AnyTableWidgetDeclaration, DeclaredTableContext } from '../../families/table/declaration.ts';
import { tableHostAdapter } from '../../families/table/hostAdapter.ts';
import { resolveTableRows, type TableProjection } from '../../families/table/model.ts';
import { encodeTableSettings, TABLE_SETTINGS_KEY, type TableSettings } from '../../families/table/settings.ts';
import { knownValue, notRecorded, zeroValue } from '../../families/value.ts';
import type { CareerRow, DecoratedDivisionsReply } from '../../generated/dashboard';
import { viewOwnedFields } from '../../families/manifest.ts';
import { OPTIONS } from './definition.ts';
import { TABLE_VIEWS } from '../manifest/table.ts';

const division = (id: string, extra: Partial<CareerRow['facts']> = {}): CareerRow => ({
  division: id, tag: 'CHI', template: '4', name: `${id}th Division`, nameMissing: null,
  facts: { medals: 2, awards: [{ medal: 0, count: 2 }], historyMissing: null, experienceRaw: '30000', level: 1, levelName: 'Trained', casualtiesTaken: 13671, ...extra },
});

const reply = (divisions: CareerRow[], partial: string | null = null): DecoratedDivisionsReply => ({
  divisions, medals: [{ key: 'medal_a', name: 'Order of the Cloud', icon: { key: 'GFX_medal_strip', frame: 2 }, iconMissing: null }],
  templates: [{ template: '4', name: 'Infantry', icon: null }], partial, missing: null, warnings: [],
});

type Stored = Readonly<Record<string, OptionValue>>;
const EVERY: DeclaredCountryFilter = { passing: null, unrecorded: [], bounds: [] };

async function hosted() {
  const entry = TABLE_VIEWS.find(item => item.kind === 'most-decorated-divisions')!;
  const spec = await entry.loadSpec();
  const declaration = spec.declaration as AnyTableWidgetDeclaration;
  const options = (stored: Stored) => resolveOptions(viewOwnedFields(OPTIONS, entry.view), stored);
  const settings = (stored: Stored): TableSettings => tableHostAdapter.settings!.resolve(declaration, null, options(stored));
  const context = (stored: Stored, filter = EVERY) => {
    const branch = { id: '1', firstHour: 0, lastHour: 1000 };
    return {
      track: { source: { id: 's', revision: 'r' }, branch, axisOffset: 0 }, branch, lastRecordedHour: 1000,
      widget: { countries: { kind: 'all' } }, options: options(stored), following: true, aux: undefined, countryTag: null, countryName: '',
      countries: { all: filter === EVERY, tags: [], holders: null, every: [], top: null, membership: null, membershipRuns: null, unobserved: [], filter },
      gateSpan: () => null, staticSpan: null, settings: settings(stored),
    } as unknown as DeclaredTableContext;
  };
  return { declaration, projection: spec.projection as TableProjection<unknown>, settings, context };
}

test('the stored sort, the row height and the dated country filter reach the product through the manifest entry', async () => {
  const table = await hosted();
  const defaults = table.settings({});
  const stored = { [TABLE_SETTINGS_KEY]: encodeTableSettings({ ...defaults, density: 'compact', sort: { column: 'casualtiesTaken', direction: 'ascending' } }) };
  const params = (raw: Stored, height?: number, filter = EVERY) => {
    const read = table.declaration.sources(table.context(raw, filter) as FamilyPlanContext, { following: true, staticSpan: null, ...(height === undefined ? {} : { height }) }).entries[0]!.read;
    return (read as unknown as { params: Record<string, unknown>; followQuery: Record<string, unknown> });
  };
  assert.deepEqual(params(stored, 280).params, { branchId: '1', countries: null, by: 'casualtiesTaken', descending: false, count: 10 }, 'ten compact rows fit in 280 px');
  assert.deepEqual(params({}, 360).params, { branchId: '1', countries: null, by: 'medals', descending: true, count: 10 });
  assert.equal(params({}, 360).followQuery.panel, 'decorated-divisions');
  const byCountry = { [TABLE_SETTINGS_KEY]: encodeTableSettings({ ...defaults, sort: { column: 'country', direction: 'ascending' } }) };
  const byDivision = { [TABLE_SETTINGS_KEY]: encodeTableSettings({ ...defaults, sort: { column: 'division', direction: 'descending' } }) };
  assert.deepEqual([params(byCountry, 360).params.by, params(byCountry, 360).params.descending, params(byDivision, 360).params.by], ['countryName', false, 'divisionName'], 'country and division sort in the product, by name at the hour');
  const germany: DeclaredCountryFilter = { passing: { GER: [[0, 99], [200, 300]] }, unrecorded: [], bounds: [0, 100, 200, 301] };
  assert.deepEqual(params({}, 360, germany).params.countries, [{ tag: 'GER', firstHour: 0, lastHour: 99 }, { tag: 'GER', firstHour: 200, lastHour: 300 }]);
});

test('rows are built for the divisions the product answered, in its order, with tooltips on demand', async () => {
  const table = await hosted();
  const answered = reply([division('7', { medals: 1 }), division('66', { medals: 3 }), division('9', { medals: 0, awards: [] })]);
  const readView = createBoundedReadView({ hour: 61000000, panels: new Map([['main', answered]]) });
  const ctx = table.context({});
  const model = table.projection.project(readView, table.declaration.projectionOptions(ctx));
  assert.equal(model.rows.length, answered.divisions.length, 'one row per answered division, none for the rest of the world');
  assert.deepEqual(resolveTableRows(model, ctx.settings).map(row => row.key), ['7', '66', '9'], 'the product order stands; the client does not re-rank');
  assert.ok(model.rows.every(row => row.tooltipExtra === undefined || typeof row.tooltipExtra === 'function'), 'medal tooltips are built when asked');
});

test('a division shows its recorded medals, experience word and casualties; an unread history is missing; unidentified awards keep their count', async () => {
  const table = await hosted();
  const answered = reply([
    division('66'),
    division('7', { medals: null, historyMissing: 'Medal history was not read', awards: [], casualtiesTaken: 0 }),
    division('8', { medals: 3, awards: [{ medal: 0, count: 1 }, { medal: null, count: 2 }] }),
  ], '2 of 40 divisions’ medals were not read at this hour; they follow the others');
  const readView = createBoundedReadView({ hour: 61000000, panels: new Map([['main', answered]]) });
  const model = table.projection.project(readView, table.declaration.projectionOptions(table.context({})));
  const [decorated, unread, partlyKnown] = model.rows;
  assert.deepEqual([decorated!.cells.medals!.value, decorated!.cells.experience!.value, decorated!.cells.casualtiesTaken!.value], [knownValue(2), knownValue('Trained'), knownValue(13671)]);
  assert.deepEqual(unread!.cells.medals!.value, notRecorded('Medal history was not read'));
  assert.deepEqual(unread!.cells.casualtiesTaken!.value, zeroValue(0));
  const strip = decorated!.cells.medals!.icons ?? [];
  assert.deepEqual(strip.map(item => [item.name, item.count, Boolean(item.url) || item.pending === true]), [['Order of the Cloud', 2, true]], 'each medal draws its own sprite');
  assert.deepEqual(partlyKnown!.cells.medals!.icons?.map(item => [item.name, item.count ?? 1, Boolean(item.url) || item.pending === true]),
    [['Order of the Cloud', 1, true], ['Medal not identified', 2, false]], 'the awards that could not be identified are drawn without art, never dropped');
  const tip = typeof decorated!.tooltipExtra === 'function' ? decorated!.tooltipExtra() : decorated!.tooltipExtra;
  assert.deepEqual(tip?.map(row => [row.label, row.value]), [['Order of the Cloud', knownValue('×2')]]);
  assert.equal(model.warnings?.[0], answered.partial, 'an order among only the divisions it could place says so');
});
