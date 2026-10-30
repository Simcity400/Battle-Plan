import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { CountryGroupReply, MembershipRun } from '../../generated/dashboard';
import type { WidgetProps } from '../../dashboard/definitions';
import type { CountrySelection } from '../../dashboard/model';
import type { BattleViewAdapter } from './BattleLogView';

const groups: CountryGroupReply = {
  branchId: '1', warnings: [], identityChanges: [], factions: [], wars: [], warStarts: [], capitulations: [],
  unobserved: { humans: [], factions: [], wars: [] }, humans: [
    { tag: 'X', firstHour: 100, lastHour: 105 }, { tag: 'Y', firstHour: 111, lastHour: 115 },
    { tag: 'X', firstHour: 125, lastHour: 130 },
  ],
  countryIdentities: [
    { tag: 'X', identities: [{ firstHour: 100, lastHour: 109, slot: 1, countryId: 11 },
      { firstHour: 120, lastHour: 139, slot: 1, countryId: 22 }] },
    { tag: 'Y', identities: [{ firstHour: 100, lastHour: 109, slot: 3, countryId: 33 },
      { firstHour: 111, lastHour: 119, slot: 1, countryId: 11 }, { firstHour: 140, lastHour: 159, slot: 3, countryId: 33 }] },
    { tag: 'Z', identities: [{ firstHour: 90, lastHour: 99, slot: 1, countryId: 11 }] },
  ],
};
const paths = [
  ['land-battles', '/ui/src/widgets/battles/BattleLogView.tsx'],
  ['construction-log', '/ui/src/widgets/construction-log/ConstructionLogWidget.tsx'],
  ['ships-sunk', '/ui/src/widgets/ships-sunk/ShipsSunkWidget.tsx'],
  ['aces', '/ui/src/widgets/aces/AcesWidget.tsx'],
  ['raid-log', '/ui/src/widgets/raid-log/RaidLogWidget.tsx'],
  ['event-feed', '/ui/src/widgets/event-feed/EventFeedWidget.tsx'],
  ['division-log', '/ui/src/widgets/division-log/DivisionLogWidget.tsx'],
] as const;
interface Predicate { tags: string[] | null; membership: MembershipRun[] | null; unobserved: { firstHour: number; lastHour: number }[] }
interface Fixture {
  actor: { tag: string; hour: number }; groups?: CountryGroupReply; error?: string;
  groupLoads: boolean[]; rows: readonly unknown[] | undefined; requests: { product: string; active: boolean; countries?: Predicate }[];
  replies: Record<string, unknown>; windows: Record<string, unknown>;
}
const host = globalThis as typeof globalThis & { __datedLogHolder?: Fixture };
const rows = (): Fixture['rows'] => host.__datedLogHolder?.rows;
function fixture(tag: string, hour: number): Fixture {
  const interval = { firstHour: 90, lastHour: 159 };
  const project = { id: '1', kind: 'building', tag, building: 0, repair: false, state: 1, toState: null,
    province: null, routeProvinces: null, baseLevel: null, targetLevel: null, costRaw: '100', createdHour: hour - 1, firstRead: hour,
    runs: [{ firstHour: hour, lastHour: 159, factories: 1, remaining: 1, repairLevels: 0 }], earlierUnknown: false,
    built: [], unaccounted: [], possible: [], added: [], end: null, firstRepairTick: null };
  const sunk = { sinkHour: hour - 1, knownAt: hour, name: 'Vessel', classIndex: 0, recordedNameIndex: null,
    victim: tag, attacker: null, victimAsOf: hour, attackerAsOf: null, placeIndex: 0, cause: 'hostile', attributionName: null, assistedKill: false };
  const ships = { branchId: '1', available: true, ...interval, firstRead: hour, unread: [], warnings: [], places: ['Sea'], recordedNames: [],
    classes: [{ token: 1, key: 'mod/ship', name: 'Ship', icon: null }], rows: [sunk] };
  const ace = { id: '1', firstSeen: 90, runs: [{ ...interval, tag, name: 0, callsign: null,
    wing: null, wingName: null, aircraft: [], alive: true, death: null }] };
  const raid = { ...interval, firstHour: hour, raider: tag, victim: null, type: 'mod/raid', province: 1, state: null,
    targetName: 'Target', icon: null, iconMissing: null, phase: 1, phaseName: 'Preparing', outcome: 0,
    outcomeName: null, launched: false, succeeded: false };
  return { actor: { tag, hour }, groups, groupLoads: [], rows: undefined, requests: [], replies: {
    'synthetic-manifest': { windows: [interval], available: true, unavailable: null, contentWarnings: [] },
    'construction-log-manifest': { costScale: 100000, windows: [interval], recorded: true, repairIcon: null, warnings: [],
      types: [{ id: 1, token: 'mod/building', name: 'Building', icon: null }] },
    'ships-sunk-manifest': { windows: [interval], classes: ships },
    'air-aces': { branchId: '1', ...interval, available: true, unavailable: null, observed: [interval], readHours: [hour], warnings: [],
      windows: [interval], aces: [{ id: '1', firstSeen: 90 }], variants: {} },
    'raids': { branchId: '1', available: true, unavailable: null, types: [{ token: 'mod/raid', name: 'Raid', missing: null }],
      phasePreparing: 1, phaseEnded: 2, outcomeNone: 0, artwork: [], episodes: [{ id: '1', start: { hour }, preparation: [], runs: [raid] }],
      series: [], readHours: [hour], warnings: [] },
    'event-feed': { branchId: '1', ...interval, windows: [], items: [], modifiers: [], texts: [], unavailable: [], warnings: [],
      rows: [{ id: '1', kind: 'ceasedToExist', category: 'warsPeace', hour: hour + 1, readHour: 159, nameHour: hour,
        exact: false, tags: [tag], item: null, amount: null }] },
    'division-log-manifest': { windows: [interval], available: true, lossesAvailable: true, templates: [], contentWarnings: [] },
    'division-log-totals': { created: 0, lost: 1, transferred: 0, warnings: [] },
  }, windows: {
    synthetic: { observed: [[90, 159]], warnings: [], contentWarnings: [] },
    'construction-log': { window: 0, interval, outputReads: [], outputUnavailable: [], projects: [project], outputs: [], warnings: [] },
    'ships-sunk': { window: 0, indices: [0], reply: ships },
    'air-aces': { branchId: '1', window: 0, ...interval, warnings: [], texts: ['Ace'], aces: [ace] },
    'division-log': { window: 0, interval, contentWarnings: [], warnings: [], rows: [{ id: '1', division: '1', readHour: 159,
      hour: hour - 1, by: false, afterHour: null, kind: 'lost', status: 'division_destroyed', tag, countryHour: hour,
      name: 'Division', nameMissing: null, icon: null, iconMissing: null, presentationHour: hour, template: null, templateName: null,
      templateMissing: null, templateHour: null, province: null, placeHour: null, placeAtEvent: false, killerTag: null, killerHour: null, killerDivision: null }] },
  } };
}

test('actual dated log callers select all native holders and send the same bounded server predicate', async () => {
  const replacements: Record<string, string> = {
    '/dashboard/useCountryGroups.ts': `export const useCountryGroups = (_runtime, _track, active) => {
      const f = globalThis.__datedLogHolder; f.groupLoads.push(active);
      return { data: f.groups, error: f.error, retry() {} };
    };`,
    '/dashboard/products.ts': `export const useProduct = (_client, request, active) => {
      const f = globalThis.__datedLogHolder;
      if (request) f.requests.push({ product: request.product, active, countries: request.params.countries });
      const data = request && f.replies[request.product];
      return { data: request?.product === 'air-aces' && request.params.view === 'roster'
        ? { manifest: data, roster: f.windows['air-aces'], readHour: f.actor.hour, preparing: [] } : data, retry() {} };
    }; export const loadProduct = () => { throw new Error('Unexpected label request'); };
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    '/dashboard/productWindows.ts': `export const useProductWindows = (_client, _identity, request) => ({
      data: new Map([[0, globalThis.__datedLogHolder.windows[request(0).product]]]), errors: new Map(), retry() {} });
      export const useWindowDisplay = (_identity, data, hour) => ({ data, hour });`,
    '/dashboard/useRaids.ts': `export const useRaids = (_widget, runtime) => ({ track: runtime.tracks[0], data: globalThis.__datedLogHolder.replies.raids });`,
    '/widgets/event-feed/useEventFeed.ts': `import { filteredEventRows } from './eventFeedData.ts';
      export const useEventFeed = (_runtime, _track, active, _hour, filter) => ({
        data: globalThis.__datedLogHolder.replies['event-feed'], identity: 'fixture', pages: [{ key: '0',
          rows: active ? filteredEventRows(globalThis.__datedLogHolder.replies['event-feed'], filter) : [] }],
        warnings: [], errors: new Map(), onWantedPages() {}, retry() {}, retryPages() {} });`,
    '/charts/log/LogList.tsx': 'export const PreparedLogCoverage = () => null; export default props => { globalThis.__datedLogHolder.rows = props.rows ?? props.pages.flatMap(page => page.rows); return null; };',
    '/charts/table/Table.tsx': 'export default props => { globalThis.__datedLogHolder.rows = props.rows; return null; };',
    '/widgets/battles/BattleLog.tsx': 'export default props => { globalThis.__datedLogHolder.rows = props.rows; return null; };',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/countryNamesAt.ts': `export const nameKey = (hour, tag) => hour + ':' + tag; export const useCountryFactsAt = () => new Map();`,
    '/dashboard/useStateNamesAt.ts': 'export const useStateNamesAt = () => () => ({ name: null, missing: null });',
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/charts/useChartTokens.ts': `export const useViewportHeight = () => 600;
      export const useChartTokens = () => ({ categories: ['#111111', '#222222', '#333333', '#444444', '#555555', '#666666'] });`,
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'dated-log-holder-fixture', enforce: 'pre',
      transform: (_source, id) => { const path = id.replaceAll('\\', '/'); const key = Object.keys(replacements).find(key => path.endsWith(key));
        return key ? replacements[key] : undefined; },
    }] });
  try {
    const adapter = { definition: { title: 'Battles', icon: 'shield', options: [] }, product: 'synthetic',
      validateManifest: (value: unknown) => value, validateWindow: (value: unknown) => value, validateRanking: (value: unknown) => value,
      rows: (_windows: unknown, _hour: number, matches: (tag: string, hour: number) => boolean) => {
        const actor = host.__datedLogHolder!.actor;
        return matches(actor.tag, actor.hour) ? [{ id: '1', provinceId: 1, windowIndex: 0, place: { name: 'Place', hour: actor.hour },
          start: { hour: 90, bound: 'exact' }, end: null, sides: [], winner: { unknown: 'Not recorded' } }] : [];
      }, rankedRows: () => [], labels: {}, measures: [{ label: 'Losses' }],
    } as unknown as BattleViewAdapter<never, never, never>;
    const props = (kind: string, countries: CountrySelection): WidgetProps => ({
      widget: { id: kind, kind, trackIds: ['main'], countries, time: { mode: 'campaign' }, options: { chips: false, totals: true, countryRole: 'victim' } },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'synthetic', revision: '1', metadata: { campaignKey: 'synthetic' } },
        branch: { id: '1', firstHour: 90, lastHour: 159 } }], clock: { getHour: () => 159, getMotion: () => 24, subscribe: () => () => {} }, client: {} },
      active: true, onDiagnostics() {},
    } as unknown as WidgetProps);
    const cases = [['Z', 95, true], ['X', 105, true], ['Y', 115, true], ['X', 125, true],
      ['Y', 145, false], ['Y', 110, false], ['X', 145, false]] as const;
    const selected: CountrySelection = { kind: 'tags', tags: ['X'] };
    for (const [kind, path] of paths) {
      const { default: Widget } = await vite.ssrLoadModule(path);
      const render = (countries: CountrySelection) => renderToStaticMarkup(createElement(Widget, { ...props(kind, countries), adapter }));
      for (const [tag, hour, expected] of cases) {
        host.__datedLogHolder = fixture(tag, hour);
        render(selected);
        assert.deepEqual(host.__datedLogHolder.groupLoads, [true], `${kind} must load the catalogue for manual choices`);
        assert.equal(rows()?.length ?? 0, Number(expected), `${kind}: ${tag} at ${hour}`);
        if (kind === 'division-log') {
          const totals = host.__datedLogHolder.requests.find(request => request.product === 'division-log-totals')!;
          assert.equal(totals.active, true);
          assert.equal(totals.countries!.membership!.some(run => run.tag === tag && run.firstHour <= hour && hour <= run.lastHour), expected);
        }
      }
      host.__datedLogHolder = fixture('Y', 115); delete host.__datedLogHolder.groups;
      assert.match(render(selected), /Loading/i, `${kind} must keep a pending identity catalogue pending`);
      assert.equal(rows(), undefined);
      host.__datedLogHolder.error = 'Native identity catalogue unavailable';
      assert.match(render(selected), /Native identity catalogue unavailable/);
      host.__datedLogHolder.groupLoads = [];
      assert.doesNotMatch(render({ kind: 'all' }), /Native identity catalogue unavailable/,
        `${kind} must bypass an inactive catalogue's retained error`);
      assert.deepEqual(host.__datedLogHolder.groupLoads, [false], `${kind} all countries needs no selection catalogue`);
      assert.equal(rows()?.length, 1);
      const humans = { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false } as const;
      host.__datedLogHolder = fixture('Y', 115); render(humans);
      assert.equal(rows()?.length, 1, `${kind} uses the actor date for hourly membership`);
      host.__datedLogHolder = fixture('Z', 95); render(humans);
      assert.equal(rows()?.length ?? 0, 0);
      render({ ...humans, membership: 'ever' });
      assert.equal(rows()?.length, 1, `${kind} ever membership retains earlier aliases of counted holders`);
      host.__datedLogHolder = fixture('Y', 115); render({ ...humans, membership: 'end' });
      assert.equal(rows(), undefined, `${kind} empty end membership stays unavailable`);
    }
    const { default: BattleView } = await vite.ssrLoadModule(paths[0][1]);
    host.__datedLogHolder = fixture('Y', 115);
    const ranked = props('land-battles', selected);
    ranked.widget.options = { view: 'biggest' };
    // Give the synthetic adapter the actual view field so the ranking request is active.
    adapter.definition = { ...adapter.definition, options: [{ key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'log',
      choices: [{ value: 'log', label: 'Log' }, { value: 'biggest', label: 'Biggest' }] }] };
    renderToStaticMarkup(createElement(BattleView, { ...ranked, adapter }));
    const countries = host.__datedLogHolder.requests.find(request => request.product === 'synthetic-ranking')!.countries!;
    assert.equal(countries.tags, null);
    for (const [tag, hour, expected] of cases) assert.equal(countries.membership!.some(run => run.tag === tag
      && run.firstHour <= hour && hour <= run.lastHour), expected, `Battle server predicate: ${tag} at ${hour}`);
  } finally { delete host.__datedLogHolder; await vite.close(); }
});
