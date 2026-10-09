import assert from 'node:assert/strict';
import { countryIdentityIndex } from '../../dashboard/countryIdentity.ts';
import { test } from 'node:test';
import { createElement, isValidElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { TableProps } from '../../charts/table/Table';
import type { CountryGroupReply } from '../../generated/dashboard';
import type { WidgetStateProps } from '../../components/WidgetState';
import type { OccupiedRow } from './occupiedStatesData';
import { countryHour, garrisonCell, occupiedAt, occupationTooltip, percent, strengthPercent, strengthBar, percentCell, stateName, validateOccupiedStatesReply } from './occupiedStatesData.ts';
import type { OccupiedStatesReply, OccupiedValues } from '../../generated/dashboard';

const value: OccupiedValues = { name: 'Mod state', nameMissing: null, owner: 'Owner with spaces', occupier: 'Dynamic tag', occupied: 'Original tag',
  resistanceRaw: '9007199254740993', complianceRaw: '0', resistanceSpeedRaw: '-1', complianceSpeedRaw: '10000',
  garrisonStrengthRaw: null, requiredManpower: 0, garrisonRequiredRaw: null, garrisonAlert: false, garrisonRed: null, assignedManpower: null,
  garrisonMissing: 'Garrison row missing', law: 'extra_mod_law', lawMissing: null };
const reply: OccupiedStatesReply = { branchId: '1', decimals: 5,
  states: [{ state: 2000, runs: [{ firstHour: 10, lastHour: 33, value }, { firstHour: 40, lastHour: 50, value }] }],
  observed: [{ firstHour: 10, lastHour: 20, readHour: 10, complete: true }, { firstHour: 21, lastHour: 33, readHour: 21, complete: true }, { firstHour: 40, lastHour: 50, readHour: 40, complete: true }],
  laws: [{ token: 'extra_mod_law', name: 'Overridden law name', icon: null, missing: 'Mod sprite missing' }], strengthMissing: 'Equipment inputs missing', warnings: [] };

test('validation accepts exact mod values and refuses malformed nested payloads', () => {
  assert.equal(validateOccupiedStatesReply(reply), reply);
  for (const data of [null, {}, { ...reply, states: [null] }, { ...reply, observed: [null] }, { ...reply, decimals: 4 },
    { ...reply, states: [{ state: 1, runs: [{ firstHour: 0, lastHour: 1, value: { ...value, resistanceRaw: '9223372036854775808' } }] }] },
    ...[{ garrisonAlert: 'yes' }, { garrisonRed: 1 }, { assignedManpower: 0.5 }].map(fields => ({ ...reply, states: [{ state: 1, runs: [{ firstHour: 10, lastHour: 20, value: { ...value, ...fields } }] }] })),
    { ...reply, observed: [{ firstHour: 10, lastHour: 20, readHour: 11, complete: true }] },
    { ...reply, observed: [{ firstHour: 10, lastHour: 33, readHour: 9, complete: true }, { firstHour: 40, lastHour: 50, readHour: 40, complete: true }] },
    { ...reply, observed: [{ firstHour: 10, lastHour: 33, readHour: 10, complete: true }, { firstHour: 40, lastHour: 50, readHour: 10, complete: true }] },
    { ...reply, laws: [...reply.laws, ...reply.laws] },
    { ...reply, states: [{ state: 1, runs: [{ firstHour: 10, lastHour: 50, value }] }] },
    { ...reply, observed: [{ firstHour: 10, lastHour: 20, readHour: 10, complete: true }, { firstHour: 21, lastHour: 33, readHour: 9, complete: true }] },
    { ...reply, states: [{ state: 1, runs: [{ firstHour: 10, lastHour: 20, value }, { firstHour: 20, lastHour: 30, value }] }] }]) {
    assert.throws(() => validateOccupiedStatesReply(data), /malformed/);
  }
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
  const held = { ...reply, states: [{ ...reply.states[0], runs: reply.states[0].runs.map(run => ({ ...run, value: { ...run.value, occupier: 'Y' } })) }] };
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
  const inventory = (occupiers: (string | null)[]): OccupiedStatesReply => ({ ...reply,
    states: occupiers.map((occupier, state) => ({ state, runs: [{ firstHour: 10, lastHour: 33, value: { ...value, occupier } }] })) });
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
  const complete: OccupiedStatesReply = { ...reply, observed: [{ firstHour: 10, lastHour: 33, readHour: 10, complete: true }],
    states: [{ state: 2000, runs: [{ firstHour: 10, lastHour: 24, value: { ...value, owner: 'X' } },
      { firstHour: 25, lastHour: 33, value: { ...value, owner: 'Y', occupier: 'X' } }] }] };
  const groups: CountryGroupReply = { branchId: '1', humans: [], factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [],
    countryIdentities: [
      { tag: 'X', identities: [{ firstHour: 0, lastHour: 9, slot: 1, countryId: 10 }, { firstHour: 20, lastHour: 33, slot: 1, countryId: 10 }] },
      { tag: 'Y', identities: [{ firstHour: 10, lastHour: 24, slot: 2, countryId: 20 }, { firstHour: 25, lastHour: 33, slot: 3, countryId: 30 }] },
      { tag: 'Other', identities: [{ firstHour: 0, lastHour: 33, slot: 4, countryId: 40 }] },
    ], unobserved: { humans: [], factions: [], wars: [] }, warnings: [] };
  type Ask = { tag: string; hour: number };
  const fixture: { data: OccupiedStatesReply; groups: CountryGroupReply; visible: OccupiedRow[]; wanted: Ask[]; sorting: Ask[];
    table?: TableProps<OccupiedRow>; state?: WidgetStateProps } = { data: complete, groups, visible: occupiedAt(complete, 25, null, 'occupier').rows, wanted: [], sorting: [] };
  const host = globalThis as typeof globalThis & { __occupiedR2?: typeof fixture };
  host.__occupiedR2 = fixture;
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': 'export const useProduct = () => ({ data: globalThis.__occupiedR2.data });',
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
        "import { useCallback, useEffect, useMemo, useState } from 'react';",
        "import { useCallback, useEffect, useMemo } from 'react'; const useState = () => [globalThis.__occupiedR2.visible, () => {}];");
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
    props.widget.countries = { kind: 'tags', tags: ['X'] };
    render();
    assert.equal(fixture.state?.kind, 'unavailable');
    assert.match('message' in fixture.state! ? fixture.state.message ?? '' : '', /native identities.*supplying read/);
    assert.equal(fixture.table, undefined);
    fixture.groups = { ...groups, countryIdentities: [...groups.countryIdentities,
      { tag: 'Alias', identities: [{ firstHour: 10, lastHour: 19, slot: 1, countryId: 10 }] }] };
    fixture.data = { ...complete, states: [...complete.states, { state: 3000,
      runs: [{ firstHour: 10, lastHour: 33, value: { ...value, occupier: 'Alias' } }] }] };
    assert.match(render(), /Some occupied states could not be matched/);
    assert.equal(currentTable()?.rows.length, 1, 'known matching rows stay visible beside an unknown endpoint');
    assert.equal(fixture.state, undefined);
    fixture.data = { ...complete, states: [{ state: 2000, runs: [{ firstHour: 10, lastHour: 33, value: { ...value, occupier: 'Other' } }] }] };
    render();
    assert.equal(currentState()?.kind, 'empty', 'a proved unrelated endpoint preserves known absence');
    fixture.data = { ...complete, states: [] };
    render();
    assert.equal(currentState()?.kind, 'empty', 'a complete empty inventory needs no endpoint attribution');
  } finally { delete host.__occupiedR2; await vite.close(); }
});

test('exact labels distinguish missing from zero and date equal consecutive reads', () => {
  assert.equal(percent('10000000'), '100%'); assert.equal(percent('9999999'), '99.99999%');
  const row = occupiedAt(reply, 25, null, 'occupier').rows[0];
  const label = (tag: string) => `Recorded name of ${tag}`;
  const tip = occupationTooltip(reply, row, 25, label);
  assert.equal(tip.rows.find(item => item.label === 'Occupied country')?.value, 'Recorded name of Original tag');
  assert.equal(tip.rows.find(item => item.label === 'Resistance speed')?.value, '−0.00001 pp/day');
  assert.equal(tip.rows.find(item => item.label === 'Compliance speed')?.value, '+0.1 pp/day');
  assert.equal(tip.rows.find(item => item.label === 'Required manpower')?.value, '0');
  assert.equal(tip.rows.find(item => item.label === 'Required garrison amount')?.value, '—');
  assert.match(tip.note!, /Read .*occupation values are read once a day/);
  assert.equal(occupationTooltip(reply, row, 21, label).note, undefined);
  const known = occupationTooltip(reply, { ...row, value: { ...row.value, garrisonStrengthRaw: '0' } }, 25, label);
  assert.equal(known.rows.find(item => item.label === 'Garrison strength')?.value, '0%');
  assert.equal(known.description, undefined);
});

test('empty state names remain authoritative while null keeps the fallback and reason', () => {
  const row = occupiedAt(reply, 25, null, 'occupier').rows[0];
  for (const [name, expected] of [['', ''], [null, 'State 2000']] as const) {
    const shown = { ...row, value: { ...row.value, name, nameMissing: name === null ? 'State name unavailable' : null } };
    assert.equal(stateName(shown), expected);
    const tip = occupationTooltip(reply, shown, 25, tag => tag);
    assert.equal(tip.title, expected);
    assert.equal(tip.rows.some(item => item.label === 'State name unavailable'), name === null);
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
  const tip = occupationTooltip(reply, { key: '2000', state: 2000, hour: 21, value: shown }, 25, tag => tag);
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
    const tooltip = occupationTooltip(reply, { key: '7', state: 7, hour: 10, value: shown }, 10, tag => tag);
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
