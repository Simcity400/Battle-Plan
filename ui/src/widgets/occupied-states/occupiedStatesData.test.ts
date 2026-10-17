import assert from 'node:assert/strict';
import { countryIdentityIndex } from '../../dashboard/countryIdentity.ts';
import { test } from 'node:test';
import { createElement, isValidElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { runAt } from '../../charts/series.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import type { TableProps } from '../../charts/table/Table';
import type { CountryGroupReply, OccupiedObservation, OccupiedSeries, OccupiedStatesManifestReply, OccupiedStatesReply } from '../../generated/dashboard';
import type { WidgetStateProps } from '../../components/WidgetState';
import type { OccupiedRow, OccupiedValues } from './occupiedStatesData';
import { countryHour, garrisonCell, occupiedAt, occupiedWindows, occupationTooltip, percent, strengthPercent, strengthBar, percentCell, stateName,
  validateOccupiedStatesManifest, validateOccupiedStatesReply, valuesAt } from './occupiedStatesData.ts';

type Run = { firstHour: number; lastHour: number; value: OccupiedValues };
const value: OccupiedValues = { name: 'Mod state', nameMissing: null, owner: 'Owner with spaces', occupier: 'Dynamic tag', occupied: 'Original tag',
  resistanceRaw: '9007199254740993', complianceRaw: '0', resistanceSpeedRaw: '-1', complianceSpeedRaw: '10000',
  garrisonStrengthRaw: null, requiredManpower: 0, garrisonRequiredRaw: null, garrisonAlert: false, garrisonRed: null, assignedManpower: null,
  garrisonMissing: 'Garrison row missing', law: 'extra_mod_law', lawMissing: null };
/** Each run as its own span, restating every value; fixed5 values hold within it. */
function series(state: number, runs: Run[], texts: string[]): OccupiedSeries {
  const text = (v: string | null) => v === null ? null : texts.includes(v) ? texts.indexOf(v) : texts.push(v) - 1;
  const fixed5 = (key: 'resistanceRaw' | 'complianceRaw' | 'resistanceSpeedRaw' | 'complianceSpeedRaw') =>
    runs.map(run => ({ firstHour: run.firstHour, lastHour: run.lastHour, raw: run.value[key], readEachHour: false }));
  const at = <T>(read: (value: OccupiedValues) => T) => runs.map(run => [run.firstHour, read(run.value)] as [number, T]);
  return { state, resistance: fixed5('resistanceRaw'), compliance: fixed5('complianceRaw'), resistanceSpeed: fixed5('resistanceSpeedRaw'), complianceSpeed: fixed5('complianceSpeedRaw'),
    name: at(v => text(v.name)), nameMissing: at(v => text(v.nameMissing)), owner: at(v => text(v.owner)), occupier: at(v => text(v.occupier)), occupied: at(v => text(v.occupied)),
    garrisonStrengthRaw: at(v => v.garrisonStrengthRaw), requiredManpower: at(v => v.requiredManpower), garrisonRequiredRaw: at(v => v.garrisonRequiredRaw),
    garrisonAlert: at(v => v.garrisonAlert), garrisonRed: at(v => v.garrisonRed), assignedManpower: at(v => v.assignedManpower),
    garrisonMissing: at(v => text(v.garrisonMissing)), law: at(v => text(v.law)), lawMissing: at(v => text(v.lawMissing)) };
}
const OBSERVED: OccupiedObservation[] = [{ firstHour: 10, lastHour: 20, readHour: 10, complete: true }, { firstHour: 21, lastHour: 33, readHour: 21, complete: true },
  { firstHour: 40, lastHour: 50, readHour: 40, complete: true }];
function windowOf(states: { state: number; runs: Run[] }[], observed = OBSERVED): OccupiedStatesReply {
  const texts: string[] = [];
  return { branchId: '1', window: 0, interval: { firstHour: 10, lastHour: 50 }, texts, states: states.map(item => series(item.state, item.runs, texts)), observed, warnings: [] };
}
const reply = windowOf([{ state: 2000, runs: [{ firstHour: 10, lastHour: 33, value }, { firstHour: 40, lastHour: 50, value }] }]);
const manifest: OccupiedStatesManifestReply = { branchId: '1', decimals: 5, windows: [{ firstHour: 10, lastHour: 50 }],
  laws: [{ token: 'extra_mod_law', name: 'Overridden law name', icon: null, missing: 'Mod sprite missing' }], strengthMissing: 'Equipment inputs missing', warnings: [] };

test('validation accepts exact mod values and refuses malformed nested payloads', () => {
  assert.equal(validateOccupiedStatesManifest(manifest), manifest);
  for (const data of [null, {}, { ...manifest, decimals: 4 }, { ...manifest, laws: [...manifest.laws, ...manifest.laws] },
    { ...manifest, windows: [{ firstHour: 10, lastHour: 50 }, { firstHour: 50, lastHour: 60 }] }]) {
    assert.throws(() => validateOccupiedStatesManifest(data), /malformed/);
  }
  assert.equal(validateOccupiedStatesReply(reply), reply);
  // The window's first observation can continue a read before it.
  const continued = { ...reply, observed: [{ ...OBSERVED[0], readHour: 5 }, ...OBSERVED.slice(1)] };
  assert.equal(validateOccupiedStatesReply(continued), continued);
  // A window answered before preparation reached it carries the laws its states hold.
  const ahead = { ...reply, laws: manifest.laws };
  assert.equal(validateOccupiedStatesReply(ahead), ahead);
  for (const laws of [[...manifest.laws, ...manifest.laws], [{ ...manifest.laws[0], icon: 'icon.png' }], 'laws'])
    assert.throws(() => validateOccupiedStatesReply({ ...reply, laws }), /malformed/);
  const state = reply.states[0];
  const withState = (fields: Record<string, unknown>) => ({ ...reply, states: [{ ...state, ...fields }] });
  for (const data of [null, {}, { ...reply, states: [null] }, { ...reply, observed: [null] }, { ...reply, states: [state, state] },
    withState({ resistance: [{ ...state.resistance[0], raw: '9223372036854775808' }, state.resistance[1]] }),
    ...[{ garrisonAlert: [[10, 'yes'], [40, false]] }, { garrisonRed: [[10, 1], [40, null]] }, { assignedManpower: [[10, 0.5], [40, null]] },
      { owner: [[10, 99], [40, 0]] }].map(withState),
    // A value not restated where a span starts would carry across the gap; none changes inside it.
    withState({ law: [state.law[0]] }), withState({ law: [state.law[0], [35, null], state.law[1]] }), withState({ compliance: [state.compliance[0]] }),
    { ...reply, interval: { firstHour: 20, lastHour: 50 } },
    { ...reply, observed: [{ ...OBSERVED[0], readHour: 11 }, ...OBSERVED.slice(1)] },
    { ...reply, observed: [OBSERVED[0], { ...OBSERVED[1], firstHour: 22 }, OBSERVED[2]] },
    { ...reply, observed: [...OBSERVED.slice(0, 2), { ...OBSERVED[2], readHour: 10 }] },
    windowOf([{ state: 1, runs: [{ firstHour: 10, lastHour: 50, value }] }]),
    windowOf([{ state: 1, runs: [{ firstHour: 10, lastHour: 20, value }, { firstHour: 20, lastHour: 30, value }] }])]) {
    assert.throws(() => validateOccupiedStatesReply(data), /malformed/);
  }
});

test('a window decodes to the recorded values at every hour, each span restating them', () => {
  const raised = { ...value, resistanceRaw: '9007199254741000', complianceRaw: '100000', garrisonStrengthRaw: '49999', garrisonRed: true, assignedManpower: 100 };
  const renamed = { ...raised, owner: 'Later owner', name: '' };
  const returned = { ...value, occupier: 'Later occupier', law: null, lawMissing: 'Group law not recorded' };
  const runs: Run[] = [{ firstHour: 10, lastHour: 20, value }, { firstHour: 21, lastHour: 24, value: raised },
    { firstHour: 25, lastHour: 33, value: renamed }, { firstHour: 40, lastHour: 50, value: returned }];
  // Each value returns at 40 to its value at 10.
  const held = (raw: string, deltas?: { deltas: string; deltaScale?: string }) => [{ firstHour: 10, lastHour: 33, raw, readEachHour: false, ...deltas },
    { firstHour: 40, lastHour: 50, raw, readEachHour: false }];
  const window: OccupiedStatesReply = { ...reply, texts: ['Mod state', 'Owner with spaces', 'Dynamic tag', 'Original tag', 'Garrison row missing',
    'extra_mod_law', 'Later owner', '', 'Later occupier', 'Group law not recorded'], states: [{ state: 2000,
    resistance: held('9007199254740993', { deltas: '11:1', deltaScale: '7' }), compliance: held('0', { deltas: '11:1' }),
    resistanceSpeed: held('-1'), complianceSpeed: held('10000'),
    name: [[10, 0], [25, 7], [40, 0]], nameMissing: [[10, null], [40, null]], owner: [[10, 1], [25, 6], [40, 1]],
    occupier: [[10, 2], [40, 8]], occupied: [[10, 3], [40, 3]], garrisonStrengthRaw: [[10, null], [21, '49999'], [40, null]],
    requiredManpower: [[10, 0], [40, 0]], garrisonRequiredRaw: [[10, null], [40, null]], garrisonAlert: [[10, false], [40, false]],
    garrisonRed: [[10, null], [21, true], [40, null]], assignedManpower: [[10, null], [21, 100], [40, null]],
    garrisonMissing: [[10, 4], [40, 4]], law: [[10, 5], [40, null]], lawMissing: [[10, null], [40, 9]] }] };
  assert.equal(validateOccupiedStatesReply(window), window);
  for (let hour = 0; hour <= 60; hour++) assert.deepEqual(valuesAt(window, window.states[0], hour), runAt(runs, hour)?.value ?? null, `hour ${hour}`);
});

test('the shown window and, while following, both neighbours are wanted', () => {
  const windows = [{ firstHour: 0, lastHour: 9 }, { firstHour: 10, lastHour: 19 }, { firstHour: 30, lastHour: 39 }];
  assert.deepEqual(occupiedWindows(windows, 15, true), { target: 1, wanted: [0, 1, 2] });
  assert.deepEqual(occupiedWindows(windows, 15, false), { target: 1, wanted: [1] });
  assert.deepEqual(occupiedWindows(windows, 0, true), { target: 0, wanted: [0, 1] });
  assert.deepEqual(occupiedWindows(windows, 25, true), { target: 1, wanted: [0, 1, 2] }, 'a gap keeps the block before it');
  assert.deepEqual(occupiedWindows(windows, 39, true), { target: 2, wanted: [1, 2] });
  assert.deepEqual(occupiedWindows(windows, -1, true), { target: -1, wanted: [] });
});

test('playback is an exact lookup; missing hours and known empty inventories differ', () => {
  assert.equal(occupiedAt(reply, 25, null, 'occupier').rows[0].value.resistanceRaw, '9007199254740993');
  assert.equal(occupiedAt(reply, 25, null, 'occupier').rows[0].hour, 21);
  assert.equal(occupiedAt(reply, 35, null, 'occupier').observation, null);
  assert.equal(occupiedAt({ ...reply, states: [] }, 25, null, 'occupied').observation?.complete, true);
  assert.equal(occupiedAt(reply, 9, null, 'occupied').rows.length, 0);
});

test('occupied filter uses the original tag, not legal owner; tags remain opaque', () => {
  assert.equal(occupiedAt(reply, 25, tag => tag === 'Original tag', 'occupied').rows.length, 1);
  assert.equal(occupiedAt(reply, 25, tag => tag === 'Owner with spaces', 'occupied').rows.length, 0);
  assert.equal(occupiedAt(reply, 25, tag => tag === 'Dynamic tag', 'occupier').rows.length, 1);
});

test('native endpoint filters follow every selected holder at the supplying occupation read', () => {
  const identities = countryIdentityIndex([
    { tag: 'X', identities: [{ firstHour: 10, lastHour: 20, slot: 1, countryId: 10 }, { firstHour: 40, lastHour: 50, slot: 1, countryId: 20 }] },
    { tag: 'Y', identities: [{ firstHour: 21, lastHour: 23, slot: 1, countryId: 10 }, { firstHour: 24, lastHour: 50, slot: 2, countryId: 30 }] },
    { tag: 'Original tag', identities: [{ firstHour: 10, lastHour: 50, slot: 3, countryId: 40 }] },
  ]);
  const selected = identities.selected(['X']);
  const matches = (tag: string, readHour: number) => { const key = identities.keyAt(tag, readHour); return key !== null && selected.has(key); };
  const held = windowOf([{ state: 2000, runs: [{ firstHour: 10, lastHour: 33, value: { ...value, occupier: 'Y' } }, { firstHour: 40, lastHour: 50, value: { ...value, occupier: 'Y' } }] }]);
  assert.equal(occupiedAt(held, 25, matches, 'occupier').rows.length, 1);
  assert.equal(occupiedAt(held, 33, matches, 'occupier').rows.length, 1);
  assert.equal(occupiedAt(held, 45, matches, 'occupier').rows.length, 0);
  assert.equal(occupiedAt(held, 25, matches, 'occupied').rows.length, 0);
  const original = identities.selected(['Original tag']);
  const originalMatches = (tag: string, readHour: number) => { const key = identities.keyAt(tag, readHour); return key !== null && original.has(key); };
  assert.equal(occupiedAt(held, 25, originalMatches, 'occupied').rows.length, 1);
  assert.equal(occupiedAt(held, 25, originalMatches, 'occupier').rows.length, 0);
});

test('legal owner labels use the shown hour while occupation endpoints keep the supplying read', () => {
  const row = occupiedAt(reply, 25, null, 'occupier').rows[0];
  assert.equal(countryHour(row, 'owner', 25), 25);
  assert.equal(countryHour(row, 'occupier', 25), 21);
  assert.equal(countryHour(row, 'occupied', 25), 21);
});

test('complete occupation inventories cannot prove filtered absence through an unresolved native endpoint', () => {
  const identities = countryIdentityIndex([
    { tag: 'X', identities: [{ firstHour: 10, lastHour: 20, slot: 1, countryId: 10 }, { firstHour: 40, lastHour: 50, slot: 1, countryId: 10 }] },
    { tag: 'Other', identities: [{ firstHour: 10, lastHour: 50, slot: 2, countryId: 20 }] },
  ]);
  const selected = identities.selected(['X']);
  const known = (tag: string, readHour: number) => identities.keyAt(tag, readHour) !== null;
  const matches = (tag: string, readHour: number) => { const key = identities.keyAt(tag, readHour); return key !== null && selected.has(key); };
  const inventory = (occupiers: (string | null)[]) => windowOf(occupiers.map((occupier, state) => ({ state, runs: [{ firstHour: 10, lastHour: 33, value: { ...value, occupier } }] })));
  const gap = occupiedAt(inventory(['X']), 25, matches, 'occupier', known);
  assert.equal(gap.observation?.complete, true);
  assert.equal(gap.rows.length, 0);
  assert.equal(gap.selectionUnknown, true);
  assert.equal(occupiedAt(inventory([null]), 25, matches, 'occupier', known).selectionUnknown, true);
  for (const tags of [[], ['Other']]) {
    const empty = occupiedAt(inventory(tags), 25, matches, 'occupier', known);
    assert.equal(empty.rows.length, 0);
    assert.equal(empty.selectionUnknown, false);
  }
  const mixed = occupiedAt(inventory(['X', 'Other']), 15, tag => tag === 'X', 'occupier', tag => tag === 'X');
  assert.equal(mixed.rows.length, 1);
  assert.equal(mixed.selectionUnknown, true);
  assert.equal(occupiedAt(inventory(['X']), 25, null, 'occupier', known).rows.length, 1, 'unfiltered values remain recorded');
});

test('actual occupation columns date hourly Owner looks and retain unknown native selection in empty states', async () => {
  const observed = [{ firstHour: 10, lastHour: 33, readHour: 10, complete: true }];
  const complete = windowOf([{ state: 2000, runs: [{ firstHour: 10, lastHour: 24, value: { ...value, owner: 'X' } },
    { firstHour: 25, lastHour: 33, value: { ...value, owner: 'Y', occupier: 'X' } }] }], observed);
  const groups: CountryGroupReply = { branchId: '1', humans: [], factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [],
    countryIdentities: [
      { tag: 'X', identities: [{ firstHour: 0, lastHour: 9, slot: 1, countryId: 10 }, { firstHour: 20, lastHour: 33, slot: 1, countryId: 10 }] },
      { tag: 'Y', identities: [{ firstHour: 10, lastHour: 24, slot: 2, countryId: 20 }, { firstHour: 25, lastHour: 33, slot: 3, countryId: 30 }] },
      { tag: 'Other', identities: [{ firstHour: 0, lastHour: 33, slot: 4, countryId: 40 }] },
    ], unobserved: { humans: [], factions: [], wars: [] }, warnings: [] };
  type Ask = { tag: string; hour: number };
  const fixture: { manifest: OccupiedStatesManifestReply; data: OccupiedStatesReply; groups: CountryGroupReply; visible: OccupiedRow[]; wanted: Ask[]; sorting: Ask[];
    windows?: number[]; request?: unknown; table?: TableProps<OccupiedRow>; state?: WidgetStateProps } = {
    manifest: { ...manifest, windows: [{ firstHour: 0, lastHour: 9 }, { firstHour: 10, lastHour: 29 }, { firstHour: 30, lastHour: 33 }] },
    data: complete, groups, visible: occupiedAt(complete, 25, null, 'occupier').rows, wanted: [], sorting: [] };
  const host = globalThis as typeof globalThis & { __occupiedR2?: typeof fixture };
  host.__occupiedR2 = fixture;
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': 'export const useProduct = () => ({ data: globalThis.__occupiedR2.manifest });',
    '/dashboard/productWindows.ts': `export const useProductWindows = (_client, _identity, request, wanted) => {
      const f = globalThis.__occupiedR2; f.windows = wanted; f.request = request(wanted[0]);
      return { data: new Map(wanted.map(index => [index, f.data])), errors: new Map(), retry() {} }; };
      export const useWindowDisplay = (identity, data, hour) => data ? { identity, data, hour } : null;`,
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__occupiedR2.groups });',
    '/charts/table/useTableHour.ts': 'export const useTableHour = clock => clock.getHour();',
    '/dashboard/playback.ts': 'export const usePlaybackSpeed = () => 0;',
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 1000;',
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = (_client, _track, wanted) => {
      globalThis.__occupiedR2.wanted = wanted; return { labelAt: hour => tag => ({ name: tag + '@' + hour, flag: tag + '-flag@' + hour }) }; };`,
    '/dashboard/countryNamesAt.ts': `export const nameKey = (hour, tag) => hour + ':' + tag;
      export const useCountryNamesAt = (_client, _track, wanted) => { globalThis.__occupiedR2.sorting = wanted;
      return new Map(wanted.map(({ hour, tag }) => [nameKey(hour, tag), tag + '@' + hour])); };`,
    '/charts/table/Table.tsx': 'export default props => { globalThis.__occupiedR2.table = props; return null; }; export const CountryCell = props => props.name; export const CellBar = props => props.children;',
    '/components/WidgetState.tsx': 'export default props => { globalThis.__occupiedR2.state = props; return props.message ?? props.label; };',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'occupied-native-dates', enforce: 'pre', transform(code, id) {
      const path = id.replaceAll('\\', '/');
      if (path.endsWith('/occupied-states/OccupiedStatesWidget.tsx')) return code.replace(
        "import { useCallback, useMemo, useState } from 'react';",
        "import { useCallback, useMemo } from 'react'; const useState = () => [globalThis.__occupiedR2.visible, () => {}];");
      return Object.entries(replacements).find(([suffix]) => path.endsWith(suffix))?.[1];
    } }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/occupied-states/OccupiedStatesWidget.tsx');
    const props = { widget: { id: 'occupation', kind: 'occupied-states', trackIds: ['main'], countries: { kind: 'all' }, time: { mode: 'follow' },
      options: { tableSort: '["owner","ascending"]' } }, runtime: { tracks: [{ id: 'main', source: { id: 'source', revision: '1', metadata: { countries: [] } },
        branch: { id: '1', firstHour: 0, lastHour: 33 }, axisOffset: 0 }], clock: { getHour: () => 25 }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const currentState = () => fixture.state;
    const currentTable = () => fixture.table;
    const render = () => { fixture.table = undefined; fixture.state = undefined; return renderToStaticMarkup(createElement(Widget, props)); };
    render();
    assert.deepEqual(fixture.windows, [0, 1, 2], 'following wants the shown block and both neighbours');
    assert.deepEqual(fixture.request, { sourceId: 'source', revision: '1', product: 'occupied-states', params: { branchId: '1', window: 0 } });
    assert.deepEqual(fixture.wanted, [{ hour: 25, tag: 'Y' }, { hour: 10, tag: 'X' }, { hour: 10, tag: 'Original tag' }]);
    assert.deepEqual(fixture.sorting, [{ hour: 25, tag: 'Y' }, { hour: 10, tag: 'X' }]);
    const row = fixture.table!.rows[0];
    for (const [role, at] of [['owner', 25], ['occupier', 10]] as const) {
      const column = fixture.table!.columns.find(column => column.key === role)!;
      assert.equal(column.value!(row), `${row.value[role]}@${at}`);
      const cell = column.render!(row);
      assert.ok(isValidElement<{ name: string; flag: string }>(cell));
      assert.equal(cell.props.name, `${row.value[role]}@${at}`);
      assert.equal(cell.props.flag, `${row.value[role]}-flag@${at}`);
    }
    const lawColumn = fixture.table!.columns.find(column => column.key === 'law')!;
    assert.equal(lawColumn.value!(row), 'Overridden law name');
    const unreadLaw = { ...row, value: { ...row.value, law: 'Unresolved mod law' } };
    assert.equal(lawColumn.value!(unreadLaw), 'Occupation law name unavailable', 'sorting uses the displayed missing title rather than the lookup key');
    const unreadCell = lawColumn.render!(unreadLaw);
    assert.ok(isValidElement<{ 'data-tooltip'?: string }>(unreadCell));
    assert.equal(unreadCell.props['data-tooltip'], 'Key: Unresolved mod law');
    assert.doesNotMatch(renderToStaticMarkup(unreadCell), />Unresolved mod law</);
    fixture.manifest = { ...fixture.manifest, laws: [{ ...fixture.manifest.laws[0], name: '' }] };
    render();
    assert.equal(fixture.table!.columns.find(column => column.key === 'law')!.value!(row), '');
    fixture.manifest = { ...fixture.manifest, laws: [{ ...fixture.manifest.laws[0], name: fixture.manifest.laws[0].token }] };
    render();
    assert.equal(fixture.table!.columns.find(column => column.key === 'law')!.value!(row), fixture.manifest.laws[0].token);
    props.widget.countries = { kind: 'tags', tags: ['X'] };
    render();
    assert.equal(fixture.state?.kind, 'unavailable');
    assert.match('message' in fixture.state! ? fixture.state.message ?? '' : '', /Occupied states unavailable for these countries/);
    assert.equal(fixture.table, undefined);
    fixture.groups = { ...groups, countryIdentities: [...groups.countryIdentities,
      { tag: 'Alias', identities: [{ firstHour: 10, lastHour: 19, slot: 1, countryId: 10 }] }] };
    fixture.data = windowOf([{ state: 2000, runs: [{ firstHour: 10, lastHour: 24, value: { ...value, owner: 'X' } },
      { firstHour: 25, lastHour: 33, value: { ...value, owner: 'Y', occupier: 'X' } }] }, { state: 3000,
      runs: [{ firstHour: 10, lastHour: 33, value: { ...value, occupier: 'Alias' } }] }], observed);
    assert.match(render(), /Some states couldn’t be matched to these countries/);
    assert.equal(currentTable()?.rows.length, 1, 'known matching rows stay visible beside an unknown endpoint');
    assert.equal(fixture.state, undefined);
    fixture.data = windowOf([{ state: 2000, runs: [{ firstHour: 10, lastHour: 33, value: { ...value, occupier: 'Other' } }] }], observed);
    render();
    assert.equal(currentState()?.kind, 'empty', 'a proved unrelated endpoint preserves known absence');
    fixture.data = windowOf([], observed);
    render();
    assert.equal(currentState()?.kind, 'empty', 'a complete empty inventory needs no endpoint attribution');
    props.widget.time = { mode: 'campaign' };
    render();
    assert.deepEqual(fixture.windows, [2], 'a fixed moment wants only its own block');
  } finally { delete host.__occupiedR2; await vite.close(); }
});

test('exact labels distinguish missing from zero and date equal consecutive reads', () => {
  assert.equal(percent('10000000'), '100%'); assert.equal(percent('9999999'), '99.99999%');
  const row = occupiedAt(reply, 25, null, 'occupier').rows[0];
  const label = (tag: string) => `Recorded name of ${tag}`;
  const tip = occupationTooltip(row, label);
  assert.equal(tip.rows.find(item => item.label === 'Occupied country')?.value, 'Recorded name of Original tag');
  assert.equal(tip.rows.find(item => item.label === 'Resistance speed')?.value, '−0.00001 pp/day');
  assert.equal(tip.rows.find(item => item.label === 'Compliance speed')?.value, '+0.1 pp/day');
  assert.equal(tip.rows.find(item => item.label === 'Required manpower')?.value, '0');
  assert.equal(tip.rows.find(item => item.label === 'Required garrison amount')?.value, '—');
  assert.equal(tip.note, undefined);
  const known = occupationTooltip({ ...row, value: { ...row.value, garrisonStrengthRaw: '0' } }, label);
  assert.equal(known.rows.find(item => item.label === 'Garrison strength')?.value, '0%');
  assert.equal(known.description, undefined);
  const failed = occupationTooltip({ ...row, value: { ...row.value, garrisonAlert: true, garrisonMissing: 'private garrison reason', lawMissing: 'private law reason' } }, label);
  assert.ok(failed.rows.some(item => item.label === 'Garrison requirement not met.'));
  assert.doesNotMatch(JSON.stringify(failed), /private|Read/);
});

test('empty state names remain authoritative while null keeps the fallback and reason', () => {
  const row = occupiedAt(reply, 25, null, 'occupier').rows[0];
  for (const [name, expected] of [['', ''], [null, 'State 2000']] as const) {
    const shown = { ...row, value: { ...row.value, name, nameMissing: name === null ? 'State name unavailable' : null } };
    assert.equal(stateName(shown), expected);
    const tip = occupationTooltip(shown, tag => tag);
    assert.equal(tip.title, expected);
    assert.equal(tip.rows.some(item => item.label === 'State name unavailable'), false);
  }
});

test('garrison text is the native amount; missing/zero hide bars, failure is !, known strength only drives the bar', () => {
  assert.deepEqual(garrisonCell(value, true), { text: '-', bar: null, red: false });
  assert.deepEqual(garrisonCell({ ...value, garrisonRequiredRaw: '0', garrisonStrengthRaw: '0' }, true), { text: '-', bar: null, red: false });
  assert.deepEqual(garrisonCell({ ...value, garrisonRequiredRaw: '-1', garrisonAlert: true }, true), { text: '!', bar: 0n, red: true });
  assert.deepEqual(garrisonCell({ ...value, garrisonRequiredRaw: '123999', garrisonStrengthRaw: '0', garrisonRed: true }, true), { text: '1.23', bar: 0n, red: true });
  assert.deepEqual(garrisonCell({ ...value, garrisonRequiredRaw: '100000' }, true), { text: '1.00', bar: null, red: false });
  assert.deepEqual(garrisonCell({ ...value, garrisonRequiredRaw: '123999', garrisonStrengthRaw: '0', garrisonRed: false }, false), { text: '1.23', bar: null, red: false });
});

test('native occupied-state percentage cells round half away from zero, then clamp to 0..100', () => {
  for (const [raw, text] of [
    ['4133540', '41%'], ['940099', '9%'], ['4149999', '41%'], ['4150000', '42%'],
    ['49999', '0%'], ['50000', '1%'], ['0', '0%'], ['9999999', '100%'],
    ['10000000', '100%'], ['10100000', '100%'], ['-50000', '0%'],
  ]) assert.equal(percentCell(raw), text, raw);
});

test('compact native cells retain exact resistance, compliance, speeds and garrison in the tooltip', () => {
  const shown = { ...value, resistanceRaw: '4133540', complianceRaw: '940099',
    resistanceSpeedRaw: '12345', complianceSpeedRaw: '-999', garrisonRequiredRaw: '123999' };
  assert.equal(percentCell(shown.resistanceRaw), '41%');
  assert.equal(percentCell(shown.complianceRaw), '9%');
  assert.equal(garrisonCell(shown, true).text, '1.23');
  const tip = occupationTooltip({ key: '2000', state: 2000, hour: 21, value: shown }, tag => tag);
  const text = (label: string) => tip.rows.find(row => row.label === label)?.value;
  assert.equal(text('Resistance'), '41.3354%');
  assert.equal(text('Compliance'), '9.40099%');
  assert.equal(text('Resistance speed'), '+0.12345 pp/day');
  assert.equal(text('Compliance speed'), '−0.00999 pp/day');
  assert.equal(text('Required garrison amount'), '1.23999');
});


test('garrison fraction uses full and half bar geometry and exact tooltip percent', () => {
  for (const [raw, percentage] of [['100000', '100%'], ['50000', '50%'], ['33333', '33.333%']]) {
    assert.equal(strengthPercent(raw), percentage);
    const shown = { ...value, garrisonRequiredRaw: '123999', garrisonStrengthRaw: raw };
    assert.equal(garrisonCell(shown, true).bar, strengthBar(raw));
    const tooltip = occupationTooltip({ key: '7', state: 7, hour: 10, value: shown }, tag => tag);
    assert.equal(tooltip.rows.find(item => item.label === 'Garrison strength')?.value, percentage);
  }
});


test('garrison bar rounds native integer percentage while tooltip retains the fraction', () => {
  assert.equal(strengthBar('100000'), 100000n);
  assert.equal(strengthBar('50000'), 50000n);
  assert.equal(strengthBar('87500'), 88000n);
  assert.equal(strengthPercent('87500'), '87.5%');
  assert.deepEqual(garrisonCell({ ...value, garrisonRequiredRaw: '-1', garrisonAlert: false, garrisonStrengthRaw: '100000' }, true), { text: '!', bar: 0n, red: true });
  assert.deepEqual(garrisonCell({ ...value, garrisonRequiredRaw: '123999', garrisonAlert: true, garrisonStrengthRaw: '0', garrisonRed: true }, true), { text: '1.23', bar: 0n, red: true });
});
