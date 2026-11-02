import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { CountryRaceViewProps } from '../../charts/race/CountryRaceView';
import { tooltipText } from '../../charts/tooltip.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import { MEMBERSHIP_PREPARING } from '../../dashboard/groups.ts';
import { assertNoMemberStates, BRANCH, gapGroups, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import type { CountryGroupReply } from '../../generated/dashboard';
import type { OccupationReserveReply } from './occupationReserveData';

/** GER holds reserve from ITA at every hour of the branch. */
const reply: OccupationReserveReply = {
  window: { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour, known: [{ firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour }],
    reads: [[BRANCH.firstHour, BRANCH.lastHour, BRANCH.firstHour]], series: [
      { name: 'GER@1|0|amount', runs: [[BRANCH.firstHour, BRANCH.lastHour, '1000', true]] },
      { name: 'GER@1|0|source', runs: [[BRANCH.firstHour, BRANCH.lastHour, '7', true]] },
    ] },
  holders: [{ tag: 'GER', slot: 1 }], sources: { 7: 'ITA@2' }, warnings: [],
};

const replacements = (key: string) => ({
  '/widgets/occupation-reserve-manpower/useOccupationReserve.ts': `export const useOccupationReserve = () => ({ track: globalThis.${key}.track,
    options: {}, data: globalThis.${key}.reply, retry() {} });`,
  '/dashboard/useCountryGroups.ts': `export const useCountryGroups = () => ({ data: globalThis.${key}.groups, retry() {} });`,
  '/dashboard/products.ts': 'export const useProduct = () => ({ data: undefined, retry() {} }); export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
  '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour(); export const usePlaybackMotion = () => 0;',
  '/dashboard/countryLooks.ts': 'export const useCountryLooks = () => ({ color: () => "#888", name: tag => tag });',
  '/charts/race/countryColours.ts': 'export const useCountryColours = () => () => "#888";',
  '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ muted: "#999" }); export const useViewportHeight = () => 1000;',
  '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
  '/charts/race/CountryRaceView.tsx': `export default props => { globalThis.${key}.view = props; return null; };`,
});

const runtime = (hour: number) => ({ tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
  clock: { getHour: () => hour, getSnapshot: () => ({ playing: false, hoursPerSecond: 0 }) }, client: {}, addWidgetBelow() {} });

test('occupation reserve of a group at an hour still being prepared is loading, not absent', async () => {
  const state = { groups: pendingGroups() as CountryGroupReply, reply, track: runtime(0).tracks[0], view: undefined as CountryRaceViewProps | undefined };
  const host = globalThis as typeof globalThis & { __occupationPending?: typeof state }; host.__occupationPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>(
    '/ui/src/widgets/occupation-reserve-manpower/OccupationReserveWidget.tsx', replacements('__occupationPending'));
  try {
    const render = () => renderToStaticMarkup(createElement(module.default, { widget: { id: 'occupation', kind: 'occupation-reserve-manpower',
      trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: {} },
      runtime: runtime(BRANCH.lastHour), active: true, onDiagnostics() {} } as unknown as WidgetProps));
    // The branch end is still being prepared: GER's membership there is not known yet.
    render();
    assert.equal(state.view?.empty, MEMBERSHIP_PREPARING);
    assert.deepEqual(state.view?.ranked, []);
    // The final reply proves there is no human player at the end: the race is genuinely empty.
    state.groups = pendingGroups({ final: true });
    render();
    assert.equal(state.view?.empty, 'No reserve manpower is held at this date.');
  } finally { delete host.__occupationPending; await close(); }
});

test('occupation reserve of a group with no member found yet says why once the moment is read, and loads until then', async () => {
  const state = { groups: gapGroups(), reply, track: runtime(0).tracks[0], view: undefined as CountryRaceViewProps | undefined };
  const host = globalThis as typeof globalThis & { __occupationGap?: typeof state }; host.__occupationGap = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>(
    '/ui/src/widgets/occupation-reserve-manpower/OccupationReserveWidget.tsx', replacements('__occupationGap'));
  try {
    assertNoMemberStates(hour => renderToStaticMarkup(createElement(module.default, { widget: { id: 'occupation', kind: 'occupation-reserve-manpower',
      trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'follow' }, options: {} },
      runtime: runtime(hour), active: true, onDiagnostics() {} } as unknown as WidgetProps)));
  } finally { delete host.__occupationGap; await close(); }
});

test('a country chosen by its old tag keeps its reserve in the race after it is renamed', async () => {
  // Native country (slot 1, id 10) is OLD through 99 and NEW from 100; it holds reserve as NEW.
  const renamed: OccupationReserveReply = { ...reply, window: { ...reply.window, series: [
    { name: 'NEW@1|0|amount', runs: [[100, BRANCH.lastHour, '1000', true]] },
    { name: 'NEW@1|0|source', runs: [[100, BRANCH.lastHour, '7', true]] },
  ] }, holders: [{ tag: 'NEW', slot: 1 }] };
  const groups = { ...pendingGroups({ final: true }), countryIdentities: [
    { tag: 'OLD', identities: [{ firstHour: BRANCH.firstHour, lastHour: 99, slot: 1, countryId: 10 }] },
    { tag: 'NEW', identities: [{ firstHour: 100, lastHour: BRANCH.lastHour, slot: 1, countryId: 10 }] },
  ] } as CountryGroupReply;
  const state = { groups, reply: renamed, track: runtime(0).tracks[0], view: undefined as CountryRaceViewProps | undefined };
  const host = globalThis as typeof globalThis & { __occupationRenamed?: typeof state }; host.__occupationRenamed = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>(
    '/ui/src/widgets/occupation-reserve-manpower/OccupationReserveWidget.tsx', replacements('__occupationRenamed'));
  try {
    renderToStaticMarkup(createElement(module.default, { widget: { id: 'occupation', kind: 'occupation-reserve-manpower',
      trackIds: ['main'], countries: { kind: 'tags', tags: ['OLD'] }, time: { mode: 'follow' }, options: {} },
      runtime: runtime(150), active: true, onDiagnostics() {} } as unknown as WidgetProps));
    assert.deepEqual(state.view?.ranked.map(entry => entry.tag), ['NEW']);
  } finally { delete host.__occupationRenamed; await close(); }
});

test('All countries displays the filtered reserve reply without loading or gating on identities', async () => {
  const tags = Array.from({ length: 1500 }, (_, index) => `Country ${String(index).padStart(4, '0')}`);
  const holders = tags.map((tag, index) => ({ tag, slot: index + 1 }));
  const groups = { ...pendingGroups({ final: true }), countryIdentities: [] } as CountryGroupReply;
  const state = { groups: groups as CountryGroupReply | undefined, error: undefined as Error | undefined,
    reply: { ...reply, holders, sources: {}, window: { ...reply.window, reads: [[100, 200, 100]],
      series: holders.map(holder => ({ name: `${holder.tag}@${holder.slot}|holder`, runs: [[100, 200, '1', true]] })) } } as OccupationReserveReply,
    track: runtime(0).tracks[0], view: undefined as CountryRaceViewProps | undefined, groupsActive: false,
    looks: [] as { tags: string[]; hour: number; exact: boolean }[] };
  const host = globalThis as typeof globalThis & { __occupationExisting?: typeof state }; host.__occupationExisting = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>(
    '/ui/src/widgets/occupation-reserve-manpower/OccupationReserveWidget.tsx', {
      ...replacements('__occupationExisting'),
      '/dashboard/useCountryGroups.ts': `export const useCountryGroups = (_runtime, _track, active) => {
        const state = globalThis.__occupationExisting; state.groupsActive = active;
        return { data: state.groups, error: state.error, retry() {} };
      };`,
      '/dashboard/countryLooks.ts': `export const useCountryLooks = (_client, _track, tags, hour, _active, _segment, _hours, exact) => {
        globalThis.__occupationExisting.looks.push({ tags, hour, exact });
        return { color: () => null, name: () => 'Country name unavailable' };
      };`,
    });
  try {
    const render = () => {
      state.view = undefined; state.looks = [];
      return renderToStaticMarkup(createElement(module.default, { widget: { id: 'occupation', kind: 'occupation-reserve-manpower',
        trackIds: ['main'], countries: { kind: 'all' }, time: { mode: 'follow' }, options: {} },
      runtime: runtime(150), active: true, onDiagnostics() {} } as unknown as WidgetProps));
    };
    const assertRace = () => {
      render();
      assert.equal(state.groupsActive, false);
      assert.deepEqual(state.view!.ranked.map(entry => entry.tag), tags);
      assert.ok(state.view!.ranked.every(entry => entry.value === 0 && entry.label === '0'));
      assert.deepEqual(state.view!.countryTags, tags);
      assert.deepEqual(state.looks, [{ tags, hour: 100, exact: true }]);
    };
    assertRace();
    state.groups = undefined;
    assertRace();
    state.error = new Error('Synthetic identity read failure');
    assertRace();
    state.error = undefined;
    state.groups = { ...groups, coverage: { firstHour: 0, lastHour: 99, complete: false }, pending: [{ firstHour: 100, lastHour: 200 }] } as CountryGroupReply;
    assertRace();
    state.reply = { ...state.reply, window: { ...state.reply.window, known: [], series: [] } };
    assert.match(render(), /Occupation manpower was not recorded at this time/);
    assert.equal(state.view, undefined);
  } finally { delete host.__occupationExisting; await close(); }
});

test('every race mode leaves nonexistent holders absent and top slots fill with real zeros', async () => {
  const tags = ['DEAD', 'BORN', 'ZERO', 'DORMANT'];
  const filtered: OccupationReserveReply = { ...reply, holders: tags.map((tag, index) => ({ tag, slot: index + 1 })), sources: {},
    window: { ...reply.window, reads: [[0, 99, 0], [100, 149, 100], [150, 200, 150]], series: [
      { name: 'DEAD@1|holder', runs: [[0, 99, '1', true]] },
      { name: 'DEAD@1|0|amount', runs: [[0, 99, '700', true]] },
      { name: 'BORN@2|holder', runs: [[100, 149, '1', true]] },
      { name: 'BORN@2|0|amount', runs: [[100, 149, '500', true]] },
      { name: 'ZERO@3|holder', runs: [[0, 200, '1', true]] },
    ] } };
  const groups = pendingGroups({ final: true, tags, humans: tags.map(tag => ({ tag, firstHour: 0, lastHour: 200 })) });
  const state = { groups, reply: filtered, track: runtime(0).tracks[0], view: undefined as CountryRaceViewProps | undefined };
  const host = globalThis as typeof globalThis & { __occupationLifecycle?: typeof state }; host.__occupationLifecycle = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>(
    '/ui/src/widgets/occupation-reserve-manpower/OccupationReserveWidget.tsx', replacements('__occupationLifecycle'));
  try {
    const modes: WidgetProps['widget']['countries'][] = [
      { kind: 'all' }, { kind: 'tags', tags }, { kind: 'selected' },
      ...[1, 5].map(count => ({ kind: 'group' as const, group: { type: 'top' as const, count }, membership: 'hour' as const, aggregate: false })),
      ...(['hour', 'end', 'ever'] as const).map(humansBy),
    ];
    for (const countries of modes) for (const hour of [50, 125, 175]) {
      const alive = hour < 100 ? ['DEAD', 'ZERO'] : hour < 150 ? ['BORN', 'ZERO'] : ['ZERO'];
      for (const selectedTag of ['DEAD', 'BORN']) {
        state.view = undefined as CountryRaceViewProps | undefined;
        renderToStaticMarkup(createElement(module.default, { widget: { id: 'occupation', kind: 'occupation-reserve-manpower',
          trackIds: ['main'], countries, time: { mode: 'follow' }, options: {} },
          runtime: { ...runtime(hour), tracks: [{ ...runtime(hour).tracks[0], selectedTag }] }, active: true, onDiagnostics() {} } as unknown as WidgetProps));
        const expected = countries.kind === 'selected' ? alive.filter(tag => tag === selectedTag)
          : countries.kind === 'group' && countries.group.type === 'top' ? alive.slice(0, countries.group.count) : alive;
        assert.deepEqual(state.view?.ranked.map(entry => entry.tag), expected, `${JSON.stringify(countries)} at ${hour}`);
        const zero = state.view?.ranked.find(entry => entry.tag === 'ZERO');
        if (expected.includes('ZERO')) assert.equal(zero?.value, 0);
      }
    }
  } finally { delete host.__occupationLifecycle; await close(); }
});

test('a group member renamed before the hour shown keeps its reserve under its earlier tag', async () => {
  // Native country (slot 1, id 10) is OLD through 99 and NEW from 100; NEW is the human player then.
  const renamed: OccupationReserveReply = { ...reply, window: { ...reply.window, series: [
    { name: 'OLD@1|0|amount', runs: [[BRANCH.firstHour, 99, '1000', true]] },
    { name: 'OLD@1|0|source', runs: [[BRANCH.firstHour, 99, '7', true]] },
  ] }, holders: [{ tag: 'OLD', slot: 1 }] };
  const groups = { ...pendingGroups({ final: true, humans: [{ tag: 'NEW', firstHour: 100, lastHour: BRANCH.lastHour }] }), countryIdentities: [
    { tag: 'OLD', identities: [{ firstHour: BRANCH.firstHour, lastHour: 99, slot: 1, countryId: 10 }] },
    { tag: 'NEW', identities: [{ firstHour: 100, lastHour: BRANCH.lastHour, slot: 1, countryId: 10 }] },
  ] } as CountryGroupReply;
  const state = { groups, reply: renamed, track: runtime(0).tracks[0], view: undefined as CountryRaceViewProps | undefined };
  const host = globalThis as typeof globalThis & { __occupationGroupRenamed?: typeof state }; host.__occupationGroupRenamed = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>(
    '/ui/src/widgets/occupation-reserve-manpower/OccupationReserveWidget.tsx', replacements('__occupationGroupRenamed'));
  try {
    for (const membership of ['end', 'ever'] as const) {
      state.view = undefined as CountryRaceViewProps | undefined;
      renderToStaticMarkup(createElement(module.default, { widget: { id: 'occupation', kind: 'occupation-reserve-manpower',
        trackIds: ['main'], countries: humansBy(membership), time: { mode: 'follow' }, options: {} },
        runtime: runtime(50), active: true, onDiagnostics() {} } as unknown as WidgetProps));
      assert.deepEqual(state.view?.ranked.map(entry => entry.tag), ['OLD'], membership);
    }
  } finally { delete host.__occupationGroupRenamed; await close(); }
});

test('reserve source descriptions request their supplying names without hover and preserve pending and absent names', async () => {
  const state = { groups: pendingGroups({ final: true }), reply: { ...reply, window: { ...reply.window,
    reads: [[BRANCH.firstHour, 99, BRANCH.firstHour], [100, BRANCH.lastHour, 100]],
  } } as OccupationReserveReply, track: runtime(0).tracks[0], view: undefined as CountryRaceViewProps | undefined,
  wanted: [] as { hour: number; tag: string }[], names: new Map<string, string | null | Error>(), refusal: undefined as string | undefined };
  const host = globalThis as typeof globalThis & { __occupationNames?: typeof state }; host.__occupationNames = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>(
    '/ui/src/widgets/occupation-reserve-manpower/OccupationReserveWidget.tsx', {
      ...replacements('__occupationNames'),
      '/widgets/occupation-reserve-manpower/useOccupationReserve.ts': `export const useOccupationReserve = () => ({ track: globalThis.__occupationNames.track,
        options: { show: 'breakdown', holder: 'GER@1' }, data: globalThis.__occupationNames.reply, retry() {} });`,
      '/dashboard/products.ts': 'export const useProduct = () => ({ data: globalThis.__occupationNames.reply, retry() {} });',
      '/dashboard/countryLooks.ts': 'export const lookHour = () => ({ hour: 0 }); export const useCountryLooks = () => ({ name: tag => tag });',
      '/dashboard/holderLooks.ts': 'export const useHolderLooks = () => ({ name: () => "Witness name", colour: () => "#888" });',
      '/dashboard/countryNamesAt.ts': `import { nameReadFrom } from './nameReads.ts'; import { RefusedError } from './queryError.ts';
        export const nameKey = (hour, tag) => JSON.stringify([hour, tag]);
        export const useCountryNamesAt = (_client, _track, wanted) => {
          const state = globalThis.__occupationNames; state.wanted.push(...wanted);
          return new Map(wanted.map(item => { const key = nameKey(item.hour, item.tag);
            return [key, nameReadFrom(state.refusal === undefined ? state.names.get(key) : new RefusedError(state.refusal), true)]; }));
        };`,
      '/dashboard/countryFlags.ts': 'export const useCountryFlagsRead = () => new Map();',
    });
  try {
    const render = (hour: number) => {
      state.wanted = [];
      renderToStaticMarkup(createElement(module.default, { widget: { id: 'occupation', kind: 'occupation-reserve-manpower', trackIds: ['main'],
        countries: { kind: 'all' }, time: { mode: 'follow' }, options: {} },
      runtime: runtime(hour), active: true, onDiagnostics() {} } as unknown as WidgetProps));
      const view = state.view!;
      return [...view.tooltip('GER', hour, 'Holder', new Map(), () => () => ({ name: 'Unused' }))!].map(tooltipText).join('; ');
    };
    assert.match(render(150), /Loading name…/);
    assert.ok(state.wanted.some(item => item.tag === 'ITA' && item.hour === 100), 'the description uses the supplying read, before any hover');
    state.names = new Map([[JSON.stringify([100, 'ITA']), 'Source at 100']]);
    assert.match(render(150), /Source at 100/);
    assert.doesNotMatch(render(150), /Country name unavailable|Loading name…/);
    assert.match(render(50), /Loading name…/, 'a different reserve read cannot borrow the later name');
    assert.ok(state.wanted.some(item => item.tag === 'ITA' && item.hour === BRANCH.firstHour));
    state.names = new Map([[JSON.stringify([BRANCH.firstHour, 'ITA']), new Error('Synthetic name failure')]]);
    assert.match(render(50), /Couldn’t load country name\./);
    assert.doesNotMatch(render(50), /Country name unavailable/, 'a failed read is not an absent name');
    state.refusal = 'Synthetic country names refused.';
    assert.match(render(50), /Synthetic country names refused\./);
    state.refusal = undefined;
    state.names = new Map([[JSON.stringify([BRANCH.firstHour, 'ITA']), null]]);
    assert.match(render(50), /Country name unavailable/, 'a completed read with no name is unavailable');
    state.reply = { ...state.reply, sources: {} };
    assert.match(render(50), /Source country unavailable/, 'an unresolved source remains absent');
  } finally { delete host.__occupationNames; await close(); }
});
