import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { SunburstProps } from '../../charts/sunburst/Sunburst';
import type { CountryGroupReply, TerritoryBreakdownReply } from '../../generated/dashboard';
import { breakdownItems as itemsAt, breakdownModel, breakdownWhole, validateBreakdownReply } from './breakdownData.ts';
import type { BreakdownLooks } from './breakdownData.ts';
import type { TerritoryMeasure, TerritoryRule } from '../../generated/dashboard';
import { groupTree, listed } from '../../charts/sunburst/layout.ts';
import { worldFilter } from '../../dashboard/groups.ts';
import { breakdownSelectedWhole } from './breakdownData.ts';
const breakdownItems = (data: TerritoryBreakdownReply, groups: CountryGroupReply, hour: number, end: number,
  measure: TerritoryMeasure, rule: TerritoryRule, looks: BreakdownLooks) => itemsAt(breakdownModel(data, measure, rule), groups, hour, end, looks);
const groups: CountryGroupReply = { branchId: '1', humans: [], factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [],
  countryIdentities: [],
  unobserved: { humans: [], factions: [], wars: [] }, warnings: [] };
const data: TerritoryBreakdownReply = { branchId: '1', regions: [{ id: 12, name: 'Modded region', missing: null }],
  observed: Array.from({ length: 8 }, () => [{ firstHour: 10, lastHour: 20 }]),
  areaUnit: null, areaReason: 'No raster', warnings: [],
  countries: [{ tag: 'opaque', runs: [{ firstHour: 10, lastHour: 20, startsSegment: true, values: [1, 1, 7, null, 1, 1, 7, null] }], segments: [{ firstHour: 10, lastHour: 20 }], membership: null }],
  worldTotals: [{ firstHour: 10, lastHour: 20, startsSegment: true, values: [1, 1, 7, null, 1, 1, 7, null] }],
  series: [{ tag: 'opaque', region: 12, missing: null, runs: [{ firstHour: 10, lastHour: 20, startsSegment: true,
    values: ['1', '7', null, '1', '7', null], stateShares: [[{ numerator: '1', denominator: 1 }], [{ numerator: '1', denominator: 1 }]] }] }] };
const looks = { country: (tag: string, hour: number) => ({ name: `${tag}@${hour}` }), faction: () => ({ name: null, reason: null }),
  colour: () => undefined, selected: () => true };
test('the whole is independent, labels are dated, and a gap never borrows a value', () => {
  assert.equal(validateBreakdownReply(data), data);
  const shape = breakdownItems(data, groups, 12, 20, 'states', 'controlled', looks);
  assert.equal(shape.items[0].exact, 1n);
  assert.equal(shape.items[0].path[1].label, 'opaque@12');
  assert.equal(shape.items[1].path[1].label, 'opaque@20');
  assert.equal(shape.items[1].present, false);
  assert.equal(breakdownItems(data, groups, 9, 20, 'states', 'controlled', looks).items[0].unknownValue, true);
  assert.equal(breakdownWhole(data, 9, 20, 'states', 'controlled').unknownValue, true);
});

test('missing region labels stay readable and tooltip notes omit backend reasons', () => {
  for (const region of [null, 12]) {
    const missing: TerritoryBreakdownReply = { ...data, regions: [{ id: 12, name: null, missing: 'Unresolved content token' }],
      series: [{ ...data.series[0], region, missing: 'State province roster absent' }] };
    const shape = breakdownItems(missing, groups, 12, 20, 'states', 'controlled', looks);
    assert.equal(shape.items[0].path[2].label, region === null ? 'Unknown region' : 'Region 12');
    assert.ok(shape.items.every(item => item.note === undefined));
    assert.equal(shape.items[0].exact, 1n);
  }
});

test('selected territory uses its native catalogue for aliases, reused tags and identity gaps', () => {
  const chosen: TerritoryBreakdownReply = { ...data,
    countries: [{ ...data.countries[0], tag: 'X', identities: [{ firstHour: 10, lastHour: 11, slot: 1, countryId: 10 }, { firstHour: 20, lastHour: 20, slot: 1, countryId: 20 }],
      segments: [{ firstHour: 10, lastHour: 11 }, { firstHour: 15, lastHour: 20 }], runs: [
        { ...data.countries[0].runs[0], lastHour: 11 },
        { ...data.countries[0].runs[0], firstHour: 12, lastHour: 14, values: [0, 0, 0, null, 0, 0, 0, null] },
        { ...data.countries[0].runs[0], firstHour: 15 },
      ] },
      { ...data.countries[0], tag: 'Y', identities: [{ firstHour: 12, lastHour: 14, slot: 1, countryId: 10 }, { firstHour: 16, lastHour: 20, slot: 2, countryId: 30 }] }],
    series: ['X', 'Y'].map(tag => ({ ...data.series[0], tag })) };
  const model = breakdownModel(chosen, 'states', 'controlled');
  const selected = (tag: string | null, hour: number) => {
    const filter = worldFilter({ kind: 'tags', tags: ['X'] }, [], groups, hour, 20, model.identities);
    const key = tag === null ? null : model.identities.keyAt(tag, hour);
    return key !== null && filter.countryIds?.has(key) === true;
  };
  const at = itemsAt(model, groups, 12, 20, { ...looks, selected });
  assert.deepEqual(at.items.map(item => [item.path[1].label, item.present, item.final]), [['Y@12', true, 0], ['X@20', false, 1]]);
  assert.equal(breakdownSelectedWhole(chosen, 12, 20, 'states', 'controlled', selected, model.identities).exact, 1n);
  const gap = breakdownSelectedWhole(chosen, 15, 20, 'states', 'controlled', selected, model.identities);
  assert.equal(chosen.countries[0].runs.at(-1)?.values[0], 1, 'the unknown gap retains its positive territory evidence');
  assert.equal(gap.exact, null);
  assert.equal(gap.unknownValue, true);
  assert.equal(gap.final, 1);
  const finalGap = breakdownSelectedWhole(chosen, 12, 15, 'states', 'controlled', selected, model.identities);
  assert.equal(finalGap.exact, 1n);
  assert.equal(finalGap.final, null);
  assert.equal(finalGap.finalUnknown, true);
});

test('selected territory distinguishes proved absent zero, active identity gaps and missing measure reads', () => {
  const absent: TerritoryBreakdownReply = { ...data, countries: [{ ...data.countries[0], tag: 'X',
    identities: [{ firstHour: 20, lastHour: 20, slot: 1, countryId: 10 }], segments: [{ firstHour: 20, lastHour: 20 }],
    runs: [{ ...data.countries[0].runs[0], lastHour: 19, values: [0, 0, 0, null, 0, 0, 0, null] },
      { ...data.countries[0].runs[0], firstHour: 20 }] }] };
  const identities = breakdownModel(absent, 'states', 'controlled').identities;
  const selected = (tag: string | null, hour: number) => tag !== null && identities.keyAt(tag, hour) !== null;
  const total = breakdownSelectedWhole(absent, 15, 20, 'states', 'controlled', selected, identities);
  assert.equal(total.exact, 0n);
  assert.equal(total.unknownValue, false);
  assert.equal(total.final, 1);
  const active = { ...absent, countries: [{ ...absent.countries[0], segments: [{ firstHour: 10, lastHour: 20 }] }] };
  assert.equal(breakdownSelectedWhole(active, 15, 20, 'states', 'controlled', selected, identities).unknownValue, true);
  const unread = { ...absent, observed: absent.observed.map(() => []) };
  assert.equal(breakdownSelectedWhole(unread, 15, 20, 'states', 'controlled', selected, identities).exact, null);
  assert.equal(breakdownSelectedWhole({ ...data, countries: [] }, 9, 20, 'states', 'controlled', selected, identities).unknownValue, true);
});

test('actual Territory breakdown forwards unknown current and final selection totals instead of empty zero', async () => {
  const unresolved: TerritoryBreakdownReply = { ...data, countries: [{ ...data.countries[0], tag: 'X',
    identities: [{ firstHour: 10, lastHour: 11, slot: 1, countryId: 10 }, { firstHour: 20, lastHour: 20, slot: 1, countryId: 10 }] }],
    series: [{ ...data.series[0], tag: 'X' }] };
  const fixture: { data: TerritoryBreakdownReply; sunburst?: SunburstProps } = { data: unresolved };
  const host = globalThis as typeof globalThis & { __breakdownR2?: typeof fixture };
  host.__breakdownR2 = fixture;
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': 'export const useProduct = () => ({ data: globalThis.__breakdownR2.data });',
    '/dashboard/useCountryGroups.ts': `export const useCountryGroups = () => ({ data: ${JSON.stringify(groups)} });`,
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 15;',
    '/dashboard/countryLooks.ts': 'export const useCountryLooks = () => ({ name: tag => tag, flag: () => undefined, flagPending: () => false, flagReason: () => undefined });',
    '/dashboard/FactionColours.tsx': 'export const useFactionColours = () => ({ names: () => ({ name: null, reason: null }), colours: new Map(), warnings: [], loading: false });',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ menu: null });',
    '/charts/sunburst/Sunburst.tsx': 'export default props => { globalThis.__breakdownR2.sunburst = props; return null; };',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'breakdown-native-unknown', enforce: 'pre', transform(_code, id) {
      const path = id.replaceAll('\\', '/'); return Object.entries(replacements).find(([suffix]) => path.endsWith(suffix))?.[1];
    } }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/territory-breakdown/TerritoryBreakdownWidget.tsx');
    const props = { widget: { id: 'territory', kind: 'territory-breakdown', trackIds: ['main'], countries: { kind: 'tags', tags: ['X'] }, time: { mode: 'follow' }, options: {} },
      runtime: { tracks: [{ id: 'main', source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: { id: '1', firstHour: 10, lastHour: 20 }, axisOffset: 0 }],
        clock: { getHour: () => 15, getSnapshot: () => ({ hoursPerSecond: 0 }) }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => { fixture.sunburst = undefined; renderToStaticMarkup(createElement(Widget, props)); return fixture.sunburst!; };
    const gap = render();
    assert.equal(gap.authoritativeTotal?.exact, null);
    assert.equal(gap.authoritativeTotal?.unknownValue, true);
    assert.equal(gap.authoritativeTotal?.final, 1);
    assert.ok(gap.unavailableReason);
    props.widget.time = { mode: 'range', firstOffset: 0, lastOffset: 5 };
    const finalGap = render();
    assert.equal(finalGap.authoritativeTotal?.final, null);
    assert.equal(finalGap.authoritativeTotal?.finalUnknown, true);
    fixture.data = { ...unresolved, countries: [{ ...unresolved.countries[0], segments: [{ firstHour: 20, lastHour: 20 }],
      runs: [{ ...unresolved.countries[0].runs[0], lastHour: 19, values: [0, 0, 0, null, 0, 0, 0, null] },
        { ...unresolved.countries[0].runs[0], firstHour: 20 }] }] };
    props.widget.time = { mode: 'follow' };
    const absent = render();
    assert.equal(absent.authoritativeTotal?.exact, 0n);
    assert.equal(absent.authoritativeTotal?.unknownValue, false);
    assert.equal(absent.unavailableReason, undefined, 'complete absence evidence still permits a known empty chart');
  } finally { delete host.__breakdownR2; await vite.close(); }
});
test('missing rosters cannot prove nonalignment, while signed negatives stay exact', () => {
  const gap = { ...groups, unobserved: { ...groups.unobserved, factions: [{ firstHour: 10, lastHour: 15 }] } };
  assert.equal(breakdownItems(data, gap, 12, 20, 'states', 'controlled', looks).items[0].path[0].label, 'Faction not recorded');
  const signed = structuredClone(data); signed.series[0].runs[0].values[1] = '-7';
  const shape = breakdownItems(signed, groups, 12, 20, 'victoryPoints', 'controlled', looks);
  assert.equal(shape.items[0].exact, -7n);
  assert.equal(shape.items[0].present, true);
  assert.equal(shape.items[1].final, -7);
  const region = groupTree(shape.items, { id: 'world', label: 'World' }).children[0].children[0].children[0];
  assert.equal(region.exact, -7n);
  assert.equal(listed(region), true);
});
test('every one of 5,000 recorded holders retains a slice and its exact amount', () => {
  const many = { ...data, series: Array.from({ length: 5_000 }, (_, i) => ({ ...data.series[0], tag: `realm ${i}` })) };
  const shape = breakdownItems(many, groups, 12, 20, 'states', 'controlled', looks);
  assert.equal(shape.items.filter(item => item.present).length, 5_000);
  assert.equal(shape.items.filter(item => item.present).reduce((sum, item) => sum + item.exact!, 0n), 5_000n);
});
test('duplicate parts and malformed signed values refuse instead of repairing history', () => {
  assert.throws(() => validateBreakdownReply({ ...data, series: [...data.series, data.series[0]] }), /malformed/);
  const broken = structuredClone(data); broken.series[0].runs[0].values[1] = '9223372036854775808';
  assert.throws(() => validateBreakdownReply(broken), /malformed/);
});

test('value changes glide within a stretch and a future region is a proven zero', () => {
  const changed = structuredClone(data);
  changed.series[0].runs = [
    { ...changed.series[0].runs[0], lastHour: 14 },
    { ...changed.series[0].runs[0], firstHour: 15, startsSegment: false },
  ];
  assert.equal(breakdownItems(changed, groups, 16, 20, 'states', 'controlled', looks).items[0].path[1].stretch, 10);
  changed.series[0].runs = [{ ...changed.series[0].runs[0], firstHour: 15, lastHour: 20 }];
  assert.equal(breakdownItems(changed, groups, 12, 20, 'states', 'controlled', looks).items[0].exact, 0n);
});

test('a missing measure breaks region motion even when identity and adjacent hours continue', () => {
  const gap = structuredClone(data);
  gap.series[0].runs = [
    { ...gap.series[0].runs[0], lastHour: 12 },
    { ...gap.series[0].runs[0], firstHour: 13, lastHour: 14, startsSegment: false, values: [null, null, null, null, null, null], stateShares: [null, null] },
    { ...gap.series[0].runs[0], firstHour: 15, startsSegment: false },
  ];
  assert.equal(breakdownItems(gap, groups, 13, 20, 'states', 'controlled', looks).items[0].unknownValue, true);
  assert.equal(breakdownItems(gap, groups, 16, 20, 'states', 'controlled', looks).items[0].path[2].stretch, 15);
});

test('regional state fractions sum exactly while independent country and world totals stay integral', () => {
  const split = structuredClone(data);
  split.regions.push({ id: 13, name: 'Other region', missing: null });
  split.series[0].runs[0].stateShares[0] = [{ numerator: '1', denominator: 3 }];
  split.series.push({ ...structuredClone(split.series[0]), region: 13,
    runs: [{ ...split.series[0].runs[0], stateShares: [[{ numerator: '2', denominator: 3 }], []] }] });
  assert.equal(validateBreakdownReply(split), split);
  const shape = breakdownItems(split, groups, 12, 20, 'states', 'controlled', looks);
  const tree = groupTree(shape.items, { id: 'world', label: 'World' }, [], breakdownWhole(split, 12, 20, 'states', 'controlled'));
  assert.equal(tree.exact, 1n);
  assert.equal(tree.children[0].children[0].exact, 1n);
  assert.equal(tree.children[0].children[0].exactDenominator, undefined);
  const third = tree.children[0].children[0].children.find(region => region.key === '12')!;
  assert.equal(third.exact, 1n); assert.equal(third.exactDenominator, 3n);
});

test('future and ended holders are absent, but a missing roster remains unknown', () => {
  const changing = structuredClone(data);
  const future = { ...structuredClone(changing.countries[0]), tag: 'future',
    segments: [{ firstHour: 15, lastHour: 20 }], runs: [
      { ...changing.countries[0].runs[0], lastHour: 14, values: [0, 0, 0, null, 0, 0, 0, null] },
      { ...changing.countries[0].runs[0], firstHour: 15 },
    ] };
  changing.countries.push(future);
  changing.series.push({ ...structuredClone(changing.series[0]), tag: 'future',
    runs: [{ ...changing.series[0].runs[0], firstHour: 15 }] });
  changing.countries[0].segments[0].lastHour = 14;
  changing.countries[0].runs = [
    { ...changing.countries[0].runs[0], lastHour: 14 },
    { ...changing.countries[0].runs[0], firstHour: 15, values: [0, 0, 0, null, 0, 0, 0, null] },
  ];
  changing.series[0].runs[0].lastHour = 14;
  assert.equal(breakdownItems(changing, groups, 12, 20, 'states', 'controlled', looks).items[1].present, false);
  assert.equal(breakdownItems(changing, groups, 18, 20, 'states', 'controlled', looks).items[0].present, false);
  assert.equal(breakdownItems(changing, groups, 9, 20, 'states', 'controlled', looks).items[0].unknownValue, true);
});

test('opaque holder tags cannot collide with anonymous holder keys after hiding the faction level', () => {
  const collisions = structuredClone(data);
  collisions.series = ['unassigned', 'holder-unknown'].map(tag => ({ ...structuredClone(data.series[0]), tag }));
  collisions.series.push({ ...structuredClone(data.series[0]), tag: null, missing: null });
  collisions.series.push({ ...structuredClone(data.series[0]), tag: null, missing: 'Country holder not recorded or ambiguous' });
  const tree = groupTree(breakdownItems(collisions, groups, 12, 20, 'states', 'controlled', looks).items,
    { id: 'world', label: 'World' }, [false, true, true]);
  assert.equal(tree.children.length, 4);
});

test('unheld territory has a separate parent from known non-aligned countries', () => {
  const unheld = { ...data, series: [{ ...data.series[0], tag: null }, data.series[0]] };
  const tree = groupTree(breakdownItems(unheld, groups, 12, 20, 'states', 'controlled', looks).items,
    { id: 'world', label: 'World' });
  assert.deepEqual(new Set(tree.children.map(node => node.label)), new Set(['Unassigned territory', 'Non-aligned']));
});

test('state-share denominators are positive, ordered and unique; fractions are never coerced', () => {
  for (const terms of [[{ numerator: '1', denominator: 0 }], [{ numerator: '-1', denominator: 2 }],
    [{ numerator: '1', denominator: 3 }, { numerator: '1', denominator: 2 }],
    [{ numerator: '1', denominator: 2 }, { numerator: '1', denominator: 2 }]]) {
    const malformed = structuredClone(data);
    malformed.series[0].runs[0].stateShares[0] = terms;
    assert.throws(() => validateBreakdownReply(malformed), /malformed/);
  }
});

test('country flag failures stay dated and anonymous holders, factions and regions remain flagless', () => {
  const pending = { name: 'Country at 12', flagPending: true };
  const failed = { name: 'Country at 20', flagReason: 'Flag unavailable at 20' };
  const withAnonymous = { ...data, series: [data.series[0], { ...data.series[0], tag: null }] };
  const shape = breakdownItems(withAnonymous, groups, 12, 20, 'states', 'controlled', {
    ...looks, country: (_tag, hour) => hour === 12 ? pending : failed,
  });
  assert.equal(shape.items[0].path[1].countryFlag, pending);
  assert.equal(shape.items[2].path[1].countryFlag, failed);
  assert.equal(shape.items[0].exact, 1n);
  assert.equal(shape.items[0].unknownValue, false);
  for (const item of shape.items) {
    assert.equal(item.path[0].countryFlag, undefined);
    assert.equal(item.path[2].countryFlag, undefined);
  }
  assert.equal(shape.items[1].path[1].countryFlag, undefined);
  assert.equal(shape.items[3].path[1].countryFlag, undefined);
  const tree = groupTree(shape.items, { id: 'world', label: 'World' }, [false, true, false]);
  assert.equal(tree.children.find(node => node.label === pending.name)?.countryFlag, pending);
});
