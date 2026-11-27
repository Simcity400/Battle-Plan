import assert from 'node:assert/strict';
import test from 'node:test';
import { countryKey } from '../../dashboard/countryKey.ts';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import { assertBarsModelValid, type BarsDataRow } from '../../families/bars/model.ts';
import { barsHostAdapter } from '../../families/bars/hostAdapter.ts';
import { lazyActionKeys } from '../../families/host/adapter.ts';
import { countryRowFromAction, recordInstalled, resolveHostInput } from '../../families/host/installed.ts';
import type { FamilyNavTuple } from '../../families/host/keyboard.ts';
import { parseTimeChartHit, timeChartHostAdapter } from '../../families/time-chart/hostAdapter.ts';
import type { IdentityProvider, InstalledPicture } from '../../families/plan.ts';
import type { EquipmentProducedRaceReply } from '../../generated/dashboard';
import { detailCountry, detailEmptyText, detailRowFromAction, equipmentProducedRaceDeclaration, equipmentProducedRaceProjection, resolvedEquipment, UNKNOWN_WORDS, type EquipmentProducedRaceOptions, type EquipmentProducedRowMeta } from './equipmentProducedRaceProjection.ts';

function fakeIdentity(names: Record<string, { name: string; color: string; tag?: string }>): IdentityProvider {
  return {
    resolve(kind, key, atHour) {
      const hit = kind === 'country' ? names[key] : undefined;
      return {
        kind,
        key,
        atHour,
        holder: { status: 'ready', value: key },
        tag: { status: 'ready', value: hit?.tag ?? key },
        name: hit ? { status: 'ready', value: hit.name } : { status: 'absent', reason: 'Name not recorded' },
        colour: hit ? { status: 'ready', value: hit.color } : { status: 'absent', reason: 'Colour not recorded' },
        flagChoice: { status: 'absent', reason: 'Flag not recorded' },
        validInterval: [0, 1000],
        nextChangeAt: Infinity,
      };
    },
  };
}

const OPTS: EquipmentProducedRaceOptions = {
  count: 'units',
  measure: 'day',
  equipmentLabel: 'Infantry Equipment',
  measureLabel: 'Per day',
  noData: null,
  group: false,
  top: 10,
};

test('production choices distinguish an empty catalogue from an unread run and keep explicit types usable', () => {
  const reason = 'Production type totals were unread in one run';
  const ask = (typesMissing: string | null, included = '') => resolvedEquipment({
    options: { count: 'units', included },
    aux: { branchId: '1', mostDelivered: null, types: [], typesMissing, warnings: [] },
  } as never).noData;
  assert.deepEqual(ask(null), { kind: 'empty', message: 'No production data in this recording.' });
  assert.deepEqual(ask(reason), { kind: 'unavailable', message: reason });
  assert.equal(ask(reason, 'sample_type'), null);
});

test('equipmentProducedRaceProjection sends group to Rust, keys by slot, keeps zeros and missing measures, and surfaces refusal', () => {
  const emptyModel = equipmentProducedRaceProjection.project(
    createBoundedReadView({ hour: displayedHour(300), panels: new Map(), identity: fakeIdentity({}) }),
    OPTS,
  );
  assertBarsModelValid(emptyModel, 300);
  assert.equal(emptyModel.rows.length, 0);

  const refused: EquipmentProducedRaceReply = {
    branchId: '1',
    hour: 300,
    firstHour: 300,
    lastHour: 300,
    count: 'units',
    measure: 'day',
    decimals: 0,
    types: ['infantry_equipment'],
    rows: [],
    readHour: 290,
    observationHour: 1,
    missing: 'Membership of this group is not recorded at this hour',
    warnings: [],
  };
  const refusedModel = equipmentProducedRaceProjection.project(
    createBoundedReadView({ hour: displayedHour(300), panels: new Map([['main', refused]]), identity: fakeIdentity({}) }),
    { ...OPTS, group: true, top: null },
  );
  assertBarsModelValid(refusedModel, 300);
  assert.equal(refusedModel.emptyKind, 'unavailable');

  const reply: EquipmentProducedRaceReply = {
    branchId: '1',
    hour: 300,
    firstHour: 290,
    lastHour: 313,
    count: 'units',
    measure: 'day',
    decimals: 0,
    types: ['infantry_equipment'],
    rows: [
      { country: 7, counted: 7, tag: 'GER', value: 420, unknown: null },
      { country: 9, counted: 9, tag: 'GER', value: 0, unknown: null },
      { country: 14, counted: 14, tag: 'FRA', value: null, unknown: 'notRecorded' },
    ],
    readHour: 290,
    observationHour: 1,
    missing: null,
    warnings: [],
  };
  const view = createBoundedReadView({
    hour: displayedHour(300),
    panels: new Map([['main', reply]]),
    identity: fakeIdentity({
      [countryKey(7)]: { name: 'German Reich', color: '#6b7280', tag: 'GER' },
      [countryKey(9)]: { name: 'West Germany', color: '#4b5563', tag: 'GER' },
      [countryKey(14)]: { name: 'France', color: '#2563eb', tag: 'FRA' },
    }),
  });
  const model = equipmentProducedRaceProjection.project(view, { ...OPTS, top: null });
  assertBarsModelValid(model, 300);
  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.deepEqual(rows.map(r => r.key), [countryKey(7), countryKey(9), countryKey(14)], 'rows keyed by slot');
  assert.equal(rows[0]!.numeric, 420);
  assert.equal(rows[1]!.value?.value.kind, 'zero', 'recorded zero preserved');
  assert.equal(rows[2]!.value?.value.kind, 'notRecorded');

  const plan = equipmentProducedRaceDeclaration.sources({
    widget: { id: 'w', kind: 'equipment-produced', trackIds: ['t'], countries: { kind: 'group', group: { type: 'faction', id: '42' }, membership: 'hour', aggregate: false }, time: { mode: 'follow' }, options: { view: 'race', count: 'ic' } },
    track: { id: 't', source: { id: 's', revision: '1' }, branch: { id: '1', firstHour: 0, lastHour: 100 }, axisOffset: 0 },
    branch: { id: '1', firstHour: 0, lastHour: 100 },
    options: { view: 'race', count: 'ic' },
    settings: {},
    countries: { tags: [], every: [], top: null, all: false, holders: null, membership: null, unobserved: [], error: null },
    aux: { branchId: '1', mostDelivered: 'infantry_equipment', types: [{ key: 'infantry_equipment', name: 'Infantry Equipment' }], warnings: [] },
  } as never, { following: true, staticSpan: null });
  const read = plan.entries[0]!.read as unknown as { params: Record<string, unknown>; buildExactParams: (base: Record<string, unknown>, hour: number) => Record<string, unknown> | null };
  const exact = read.buildExactParams(read.params, 300);
  assert.deepEqual(exact?.group, { type: 'faction', id: '42', membership: 'hour' }, 'group selection sent to Rust');
});

test('a race row\'s detail asks for the country the row was counted for, also after it changed tag, and says when its window cannot be counted', () => {
  const reply: EquipmentProducedRaceReply = {
    branchId: '1', hour: 300, firstHour: 290, lastHour: 313, count: 'units', measure: 'total', decimals: 0, types: ['infantry_equipment'],
    rows: [{ country: 7, counted: 7, tag: 'DEU', value: 8, unknown: null }, { country: 7, counted: 'DEU', tag: 'DEU', value: null, unknown: 'windowIncomplete' }], readHour: 290, observationHour: 120, missing: null, warnings: [],
  };
  const view = createBoundedReadView({ hour: displayedHour(300), panels: new Map([['main', reply]]), identity: fakeIdentity({ [countryKey(7)]: { name: 'Germany', color: '#6b7280', tag: 'DEU' } }) });
  const [ranked, asked] = equipmentProducedRaceProjection.project(view, { ...OPTS, measure: 'total' }).rows.filter((r): r is BarsDataRow => r.kind === 'data').map(r => r.meta as unknown as EquipmentProducedRowMeta);
  assert.equal(ranked!.tag, 'DEU', 'the detail is titled by the tag shown');
  assert.equal(detailCountry(ranked!), 7, 'and asks for the slot the race counted, not the tag it has now');
  assert.equal(detailCountry(asked!), 'DEU', 'a row counted for a tag opens the same tag\'s history, not the slot it shows');

  const icTotal = (reason: 'windowIncomplete' | 'observationRestarted' | null) => ({ value: null, reason });
  assert.equal(detailEmptyText({ models: [], icTotal: icTotal('windowIncomplete') }, 300), UNKNOWN_WORDS.windowIncomplete, 'a window it cannot count is not "nothing delivered"');
  assert.equal(detailEmptyText({ models: [], icTotal: icTotal('observationRestarted') }, 300), UNKNOWN_WORDS.observationRestarted);
  assert.equal(detailEmptyText({ models: [], icTotal: { value: null, reason: 'notRecorded' } }, 300), UNKNOWN_WORDS.notRecorded, 'production not recorded is not "nothing delivered"');
  assert.match(detailEmptyText({ models: [], icTotal: { value: 0, reason: null } }, 300)!, /^Nothing delivered at /);
  assert.equal(detailEmptyText({ models: [{} as never], icTotal: icTotal('windowIncomplete') }, 300), null, 'models are listed, each with its own amount');
});

test('activating a dated chart plot coordinate does not open the country drawer, while activating a country row does', () => {
  const basePicture = (owner: string, nav: readonly FamilyNavTuple[]): InstalledPicture => ({
    id: 1,
    owner,
    generation: { seek: 1, reset: 0, binding: 1 },
    factRevisions: {},
    validInterval: [0, 1000],
    through: 1000,
    hour: displayedHour(500),
    viewport: { width: 400, height: 200, dpr: 1, scrollLeft: 0, scrollTop: 0 },
    optionsRevision: '1',
    themeRevision: '1',
    actionKeys: lazyActionKeys(nav, displayedHour(500)),
    accessibleDescription: null,
  });

  const chartPicture = basePicture('produced:lines', []);
  const chartRecord = recordInstalled(chartPicture, { meta: { transform: { axisFrom: 0, axisTo: 1000 } } }, null);
  const plotHit = parseTimeChartHit('plot:320')!;
  const plotEffect = resolveHostInput(
    timeChartHostAdapter,
    'activate',
    { key: plotHit.key, hit: plotHit, picture: chartPicture },
    { current: chartRecord, held: false, expired: false },
    { expansion: {}, takesToggles: false, regions: false, menu: false },
    () => null,
  );
  assert.equal(plotEffect?.kind, 'activate');
  if (plotEffect?.kind !== 'activate') return;
  assert.deepEqual(plotEffect.action.target, { kind: 'chart-coordinate', coordinate: 'plot', hour: displayedHour(320) });
  assert.equal(detailRowFromAction(plotEffect.action), null, 'plot coordinate must not open equipment-produced country drawer');
  assert.equal(countryRowFromAction(plotEffect.action), null, 'plot coordinate must not open shared country drawer');

  const lineHit = parseTimeChartHit('line:AAA@320')!;
  const lineEffect = resolveHostInput(
    timeChartHostAdapter,
    'activate',
    { key: lineHit.key, hit: lineHit, picture: chartPicture },
    { current: chartRecord, held: false, expired: false },
    { expansion: {}, takesToggles: false, regions: false, menu: false },
    () => null,
  );
  assert.equal(lineEffect?.kind, 'activate');
  if (lineEffect?.kind !== 'activate') return;
  assert.equal(detailRowFromAction(lineEffect.action), null, 'chart series line is not a country row');
  assert.equal(countryRowFromAction(lineEffect.action), null, 'chart series line is not a country row');

  const rowMeta: EquipmentProducedRowMeta = { tag: 'AAA', key: countryKey(7), counted: countryKey(7) };
  const rowNav: FamilyNavTuple[] = [[rowMeta.key, 'Alpha Republic', 0, -1, '', rowMeta]];
  const barsPicture = basePicture('produced:race', rowNav);
  const barsRecord = recordInstalled(barsPicture, { nav: rowNav }, null);
  const rowHit = barsHostAdapter.parseHit!(`row:${rowMeta.key}`)!;
  const rowEffect = resolveHostInput(
    barsHostAdapter,
    'activate',
    { key: rowHit.key, hit: rowHit, picture: barsPicture },
    { current: barsRecord, held: false, expired: false },
    { expansion: {}, takesToggles: false, regions: false, menu: false },
    () => null,
  );
  assert.equal(rowEffect?.kind, 'activate');
  if (rowEffect?.kind !== 'activate') return;
  assert.deepEqual(rowEffect.action.target, { kind: 'row', key: rowMeta.key });
  assert.deepEqual(detailRowFromAction(rowEffect.action), rowMeta, 'country row opens equipment-produced drawer');
  assert.deepEqual(countryRowFromAction(rowEffect.action), { tag: 'AAA', key: rowMeta.key }, 'country row opens shared country drawer');
  assert.equal(detailRowFromAction({ ...plotEffect.action, meta: rowMeta }), null, 'target.kind rejects chart-coordinate even with row-shaped meta');
  assert.equal(countryRowFromAction({ ...lineEffect.action, meta: rowMeta }), null, 'target.kind rejects chart-series even with row-shaped meta');
});
