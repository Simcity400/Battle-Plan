import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { countryDrillTagAt } from '../../dashboard/countryIdentity.ts';
import { datedCountrySpans, readDatedCountry } from '../../dashboard/datedCountry.ts';
import { keyId } from '../../dashboard/keyedSeriesData.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { setOption, setToken } from '../../dashboard/options.ts';
import { PlaybackController } from '../../dashboard/playback.ts';
import { holderSelection, type Gate, type Holder } from '../../dashboard/regionGate.ts';
import { axisAt, FOLLOW, regionOutside, recordedHour } from '../../dashboard/regionTime.ts';
import { followSpan, watchBoundary } from '../../dashboard/widgetTime.ts';
import { hitTestTimeChart } from '../../families/time-chart/hitTest.ts';
import { parseTimeChartHit } from '../../families/time-chart/hostAdapter.ts';
import { layoutTimeChart } from '../../families/time-chart/layout.ts';
import type { TimeChartModel } from '../../families/time-chart/model.ts';
import type { CountryIdentityHistory } from '../../generated/dashboard';
import { demandHistoryRange } from '../fuel/fuelConsumption.ts';
import { detailWindows } from '../fuel/fuelDrill.ts';
import '../definitions.ts';
import { FAMILY_VIEW_UI, familyViewOptionFields, widgetViewOptions } from '../manifest.ts';
import { OPTIONS as FUEL_BRANCHES_OPTIONS } from './definition.ts';
import {
  branchDrillCanFollow,
  branchDrillDetailSubject,
  branchDrillSelectionKey,
  parseBranchFamily,
  resolveBranchDrillOpen,
} from './fuelBranchesDrill.ts';

const H1936 = 60_794_688;
const measure = (text: string) => text.length * 7;

const RENAME_AND_REUSE_HISTORIES: readonly CountryIdentityHistory[] = [
  {
    tag: 'GER',
    identities: [
      { firstHour: H1936, lastHour: H1936 + 199, slot: 1, countryId: 10 },
      { firstHour: H1936 + 250, lastHour: H1936 + 500, slot: 2, countryId: 20 },
    ],
  },
  {
    tag: 'DEU',
    identities: [
      { firstHour: H1936 + 200, lastHour: H1936 + 500, slot: 1, countryId: 10 },
    ],
  },
];

const TRACK = {
  id: 'main',
  axisOffset: 0,
  source: { id: 'src', revision: '1', metadata: { campaignKey: 'camp' } },
  branch: {
    id: 'b0',
    firstHour: H1936,
    lastHour: H1936 + 5000,
    gaps: [{ firstHour: H1936 + 220, lastHour: H1936 + 240, reason: 'Collector paused during reload' }],
  },
} as unknown as BoundTrack;

test('fuel-branches shares the reached country option across bars and over-time views without switching holders on view change', () => {
  const entry = FAMILY_VIEW_UI['fuel-branches'];
  assert.ok(entry, 'fuel-branches must be registered in FAMILY_VIEW_UI');
  assert.equal(entry.views, undefined, 'wrapper applies to both bars and time views');
  const token = setToken(JSON.stringify({ campaign: 'camp', branch: 'b0', tag: 'GER', originOffset: 50 }));
  assert.ok(setOption(FUEL_BRANCHES_OPTIONS, {}, 'country', token));

  const barsFields = familyViewOptionFields('fuel-branches', 'bars');
  const savedInBars = setOption(barsFields, { view: 'bars' }, 'country', token);
  assert.ok(savedInBars, 'bars view accepts the chosen country holder');
  assert.equal(savedInBars.country, token, 'stored under shared country key rather than country@bars');

  const afterSwitch = { ...savedInBars, view: 'time' };
  const barsResolved = widgetViewOptions('fuel-branches', savedInBars, 'bars');
  const timeResolved = widgetViewOptions('fuel-branches', afterSwitch, 'time');
  assert.equal(barsResolved.country, token);
  assert.equal(timeResolved.country, token, 'switching bars -> time preserves the chosen holder');
  assert.equal(
    widgetViewOptions('division-equipment', { 'country@equipment': token }, 'equipment').country,
    token,
    'existing division-equipment dashboards keep reading legacy scoped country@equipment holders',
  );
  assert.equal(
    widgetViewOptions('division-equipment', { 'country@template': token }, 'template').country,
    token,
    'existing division-equipment dashboards keep reading legacy scoped country@template holders',
  );

  const dated = readDatedCountry(timeResolved.country, 'camp', 'b0');
  assert.ok(dated);
  const spans = datedCountrySpans(RENAME_AND_REUSE_HISTORIES, dated, H1936, H1936 + 500);
  assert.ok(spans);
  assert.equal(countryDrillTagAt(spans, H1936 + 300), 'DEU', 'over-time view still follows original holder DEU after view switch at hour 300');
});

test('parseBranchFamily resolves standard military branches from bars keys and time-chart keyIds while leaving plot, gap and modded indices unfiltered', () => {
  assert.equal(parseBranchFamily('0'), 'family:0');
  assert.equal(parseBranchFamily('1'), 'family:1');
  assert.equal(parseBranchFamily('2'), 'family:2');
  assert.equal(parseBranchFamily(keyId({ tag: 'slot:1', part: '2' })), 'family:2');
  assert.equal(parseBranchFamily(keyId({ tag: 'GER', part: '1' })), 'family:1');
  assert.equal(parseBranchFamily('3'), null);
  assert.equal(parseBranchFamily(keyId({ tag: 'GER', part: '3' })), null);
  assert.equal(parseBranchFamily('plot:500'), null);
  assert.equal(parseBranchFamily(`gap:0@${H1936 + 230}`), null);
  assert.equal(parseBranchFamily(null), null);
});

test('dated holder preserves the original country across rename and tag reuse in both bars and over-time views', () => {
  const originalHolder = { tag: 'GER', origin: H1936 + 50 };
  const clickHour = H1936 + 300;
  const barsOpen = resolveBranchDrillOpen({
    active: true,
    holder: originalHolder,
    extent: [H1936, H1936 + 500],
    hour: clickHour,
    subject: '1',
  });
  const timeOpen = resolveBranchDrillOpen({
    active: true,
    holder: originalHolder,
    extent: [H1936, H1936 + 500],
    hour: clickHour,
    subject: keyId({ tag: 'slot:1', part: '2' }),
  });
  assert.deepEqual(barsOpen, { holder: originalHolder, hour: clickHour, initialLevel: 'family:1' });
  assert.deepEqual(timeOpen, { holder: originalHolder, hour: clickHour, initialLevel: 'family:2' });
  for (const opened of [barsOpen!, timeOpen!]) {
    const selection = holderSelection(RENAME_AND_REUSE_HISTORIES, TRACK, opened.holder);
    assert.ok(selection);
    const spans = datedCountrySpans(RENAME_AND_REUSE_HISTORIES, selection, H1936, H1936 + 500);
    assert.ok(spans);
    assert.equal(countryDrillTagAt(spans, clickHour), 'DEU', 'drill follows renamed original holder rather than the new GER holder');
    assert.equal(countryDrillTagAt(spans, H1936 + 100), 'GER', 'pre-rename hour still resolves to original GER');
  }
});

test('selection scope stays stable across rename, gaps and failures while resetting when choosing another holder or country', () => {
  const base = {
    sourceId: 'src',
    revision: '1',
    branchId: 'b0',
    chosen: 'GER',
    savedSelection: { campaign: 'camp', branch: 'b0', tag: 'GER', originOffset: 50 },
    branchFirstHour: H1936,
    resolvedHolder: { tag: 'GER', origin: H1936 + 50 },
    drillHolder: { tag: 'GER', origin: H1936 + 50 },
    extent: [H1936, H1936 + 500] as const,
  };
  const openKey = branchDrillSelectionKey(base);
  assert.equal(
    branchDrillSelectionKey({ ...base, resolvedHolder: null }),
    openKey,
    'entering a gap or absent date keeps the open drill mounted',
  );
  assert.equal(
    branchDrillSelectionKey({ ...base, savedSelection: null, resolvedHolder: null }),
    openKey,
    'unsaved selection keeps the open drill mounted while parent holder is unavailable',
  );
  assert.notEqual(
    branchDrillSelectionKey({
      ...base,
      savedSelection: { campaign: 'camp', branch: 'b0', tag: 'GER', originOffset: 250 },
      resolvedHolder: { tag: 'GER', origin: H1936 + 250 },
    }),
    openKey,
    'picking the reused GER holder resets the open drill',
  );
  assert.notEqual(
    branchDrillSelectionKey({ ...base, chosen: 'FRA', savedSelection: null, resolvedHolder: { tag: 'FRA', origin: H1936 } }),
    openKey,
    'switching countries resets the open drill',
  );
  const slotKey = branchDrillSelectionKey({ ...base, chosen: 'slot:1', savedSelection: null, resolvedHolder: null });
  assert.notEqual(
    branchDrillSelectionKey({ ...base, chosen: 'slot:2', savedSelection: null, resolvedHolder: null }),
    slotKey,
  );
});

test('clicking a recording gap carries the clicked gap hour and opens pinned on the gap refusal', () => {
  const hour = H1936 + 400;
  const model: TimeChartModel = {
    kind: 'fuel-branches',
    measureTitle: 'Fuel received',
    mode: 'lines',
    hour,
    extent: [H1936, hour],
    yAxis: { format: 'percent' },
    series: [],
    gaps: [{ firstHour: H1936 + 220, lastHour: H1936 + 240, reason: 'Collector paused during reload' }],
  };
  const layout = layoutTimeChart(model, 600, 260, '12px sans-serif', measure);
  assert.equal(layout.gaps.length, 1);
  const placed = layout.gaps[0]!;
  const hitKey = hitTestTimeChart(model, layout, [(placed.x0 + placed.x1) / 2, 100]);
  assert.equal(hitKey, `gap:0@${H1936 + 230}`);
  const parsed = parseTimeChartHit(hitKey);
  assert.equal(parsed?.hour, H1936 + 230);
  const opened = resolveBranchDrillOpen({
    active: true,
    holder: { tag: 'GER', origin: H1936 + 50 },
    extent: [H1936, H1936 + 500],
    hour: parsed!.hour!,
    subject: parsed!.subject,
  });
  assert.deepEqual(opened, { holder: { tag: 'GER', origin: H1936 + 50 }, hour: H1936 + 230, initialLevel: null });
  assert.equal(recordedHour(TRACK, opened!.hour), null);
  assert.equal(
    branchDrillDetailSubject(null, opened!.holder, true),
    `GER@${H1936 + 50}`,
    'detail window keeps a non-null subject in a gap so the refusal remains visible',
  );
  assert.equal(branchDrillDetailSubject(null, opened!.holder, false), null);
});

test('followed consumer history anchors to the displayed fuel reply hour rather than the end of the recording', () => {
  const extent: readonly [number, number] = [H1936, H1936 + 5000];
  const pausedHour = H1936 + 200;
  const [start, end] = demandHistoryRange(extent, pausedHour, 720);
  assert.deepEqual([start, end], [H1936, pausedHour]);
  const windows = detailWindows(extent, [start, end], pausedHour, 720);
  assert.deepEqual(
    windows,
    [Math.floor(pausedHour / 720)],
    'followed consumer history requests the window at the paused reply hour, not the end of the recording',
  );
  assert.notEqual(windows[0], Math.floor(extent[1] / 720));
});

test('paused seek from a recording gap into coverage wakes the coverage watch and enables Follow', () => {
  const clock = new PlaybackController();
  clock.setBounds({ firstHour: H1936, lastHour: H1936 + 5000 });
  clock.seek(H1936 + 230);
  const widget = { id: 'w1', trackIds: ['main'], time: { mode: 'follow' }, countries: { kind: 'tags', tags: ['GER'] } } as unknown as WidgetSpec;
  const followRegion = { widget, action: FOLLOW };
  const holder = { tag: 'GER', origin: H1936 + 50 };
  let notifications = 0;
  const stop = watchBoundary(clock, followSpan(TRACK), () => { notifications++; });
  try {
    const outsideBefore = regionOutside(followRegion, TRACK, [TRACK], axisAt(clock.getHour()));
    assert.ok(outsideBefore, 'paused clock inside a gap reports outside coverage');
    assert.equal(branchDrillCanFollow(outsideBefore === null, holder, null), false);

    clock.seek(H1936 + 300);
    assert.equal(notifications, 1, 'paused seek across the gap boundary notifies coverage listeners immediately');
    const outsideAfter = regionOutside(followRegion, TRACK, [TRACK], axisAt(clock.getHour()));
    assert.equal(outsideAfter, null);
    assert.equal(branchDrillCanFollow(outsideAfter === null, holder, null), true);
  } finally {
    stop();
  }
});

test('DeclaredFamilyWidget preserves string action.meta while passing countryHolder separately', async () => {
  let hostActivate: ((action: { installedId: number; owner: string; subject: string; hour: number; meta?: unknown }) => void) | undefined;
  const holder: Holder = { tag: 'GER', origin: H1936 + 50 };
  const harness = {
    holder,
    captureHostActivate(fn: typeof hostActivate) {
      hostActivate = fn;
    },
  };
  const hostGlobal = globalThis as typeof globalThis & { __dfwMetaHarness?: typeof harness };
  hostGlobal.__dfwMetaHarness = harness;

  const vite = await createServer({
    configFile: false,
    optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{
      name: 'dfw-meta-test',
      enforce: 'pre',
      transform(code, id) {
        const norm = id.replaceAll('\\', '/');
        if (norm.endsWith('/families/host/DeclaredFamilyWidget.tsx')) {
          return code
            .replace("from './useFamilyPlan.ts'", "from 'dfw-test-plan'")
            .replace("from './FamilyHost.tsx'", "from 'dfw-test-host'")
            .replace("from './theme.ts'", "from 'dfw-test-theme'")
            .replace("from '../art/react.ts'", "from 'dfw-test-art'")
            .replace("from '../../dashboard/widgetFrame.tsx'", "from 'dfw-test-frame'")
            .replace("from '../../components/Popover'", "from 'dfw-test-popover'")
            .replace("from '../regions/useRegionList.ts'", "from 'dfw-test-regions'");
        }
      },
      resolveId(id) {
        if (id.startsWith('dfw-test-')) return `\0${id}`;
      },
      load(id) {
        if (id === '\0dfw-test-frame') return `export const useWidgetSkeleton = () => {};`;
        if (id === '\0dfw-test-theme') return `export const useFamilyTheme = () => ({ theme: {} });`;
        if (id === '\0dfw-test-art') return `export const useArtHandleResolutions = () => ({ onArtDemands() {} });`;
        if (id === '\0dfw-test-popover') return `export const usePopover = () => ({ popup: { current: null }, openAt() {} }); export const Popover = () => null;`;
        if (id === '\0dfw-test-regions') return `export const useRegionList = () => ({ list: [], error: null, retry() {} });`;
        if (id === '\0dfw-test-plan') {
          return `export const useFamilyPlan = () => ({
            track: null, options: {}, following: true, owner: 'o', ctx: null, displayedCtx: null,
            recorded: () => null, countryHolder: globalThis.__dfwMetaHarness.holder, displayedExtra: undefined,
            plan: null, subscriptions: [], allowEmptyFollow: false, followRefused: null, staticBundle: null,
            staticProof: { kind: 'none' }, staticHeld: false, pagedRows: undefined, onRowWindow() {},
            tiledSeries: {}, onSeriesViewport() {}, retryTiles() {}, onHoverHour() {}, hovers: {},
            exactReadView: undefined, bodyStatus: { kind: 'ready' }, bannerFailure: null, nameFailureDetail: undefined,
            groupWarnings: [], moment: 0, bounds: null, pin: null, offset: 0, loadingLabel: '',
            identitySnapshot: null, identityProvider: undefined, artProvider: {}, onIdentityDemands() {}, measureViewport() {},
          });`;
        }
        if (id === '\0dfw-test-host') {
          return `import { createElement } from 'react';
            export default function StubHost(props) {
              globalThis.__dfwMetaHarness.captureHostActivate(props.onActivate);
              return createElement('div');
            }`;
        }
      },
    }],
  });

  try {
    const { default: DeclaredFamilyWidget } = await vite.ssrLoadModule('/ui/src/families/host/DeclaredFamilyWidget.tsx') as typeof import('../../families/host/DeclaredFamilyWidget.tsx');
    const received: Array<{ meta: unknown; holder: Holder | null | undefined }> = [];
    renderToStaticMarkup(createElement(DeclaredFamilyWidget, {
      adapter: { family: 'table' } as never,
      declaration: {
        family: 'table',
        kind: 'special-projects',
        label: 'Special projects',
        subject: 'special projects',
        emptyCampaign: '',
        countries: { mode: 'single' },
        sources: () => ({ owner: 'o', entries: [] }),
        projection: {} as never,
        projectionOptions: () => ({}),
      },
      widget: { id: 'w', kind: 'special-projects', trackIds: [], countries: { kind: 'tags', tags: ['GER'] }, time: { mode: 'follow' } } as never,
      options: {},
      runtime: { tracks: [], clock: {}, client: {} } as never,
      active: true,
      mapOptions: {} as never,
      headerSlot: null,
      onDiagnostics: () => {},
      onActivate: (action, activeHolder) => {
        received.push({ meta: action.meta, holder: activeHolder });
      },
    }));
    assert.ok(hostActivate, 'FamilyHost received onActivate');
    hostActivate!({ installedId: 1, owner: 'o', subject: 'proj', hour: H1936 + 10, meta: 'project:42' });
    assert.deepEqual(received, [{ meta: 'project:42', holder }], 'string action.meta is preserved intact alongside separate holder arg');
  } finally {
    delete hostGlobal.__dfwMetaHarness;
    await vite.close();
  }
});

test('FuelBranchesWidget keeps the detail window mounted across refusals, uses the chart activation holder, keeps pinned playhead and followed extent independent of parent bounds, and updates Follow', async () => {
  const clock = new PlaybackController();
  clock.setBounds({ firstHour: H1936, lastHour: H1936 + 5000 });
  clock.seek(H1936 + 230);

  const holder = { tag: 'GER', origin: H1936 + 50 };
  const shownCalls: Array<{ breakdown: string; hour: number; country: { tag: string; origin: number } | null | undefined }> = [];
  let capturedActivate: ((action: { hour: number; subject?: string }, activeHolder?: Holder | null) => void) | undefined;
  let breakdownProps: { playhead: number | null; hour: number | null; extent: readonly [number, number] } | null = null;
  let breakdownMounts = 0;

  const makeGate = (stage: Gate['stage'], tag: string | null, gatedHour = H1936 + 300): Gate => ({
    region: { widget: {} as WidgetSpec, action: FOLLOW },
    track: TRACK,
    active: true,
    hour: stage.stage === 'open' ? gatedHour as never : null,
    lastRecorded: (H1936 + 5000) as never,
    window: null,
    outside: stage.stage === 'outside' ? stage.outside : null,
    stage,
    open: stage.stage === 'open',
    country: {
      chosen: 'GER',
      tag,
      absent: stage.stage === 'absent',
      reason: stage.stage === 'absent' ? stage.reason : null,
      holders: null,
      pick() {},
      retry() {},
      historyHeld: true,
      aliasAt: () => tag,
      edges: [],
      holder,
      history: { key: 'slot:1', spans: [{ firstHour: H1936, lastHour: H1936 + 5000 }] },
      aliases: null,
    },
    members: null,
    groups: {} as never,
    held: null,
    hourAt: () => null,
    recorded: () => null,
    within: () => null,
    span: () => null,
    follow: () => null,
    checkFollow: () => ({ query: null, refused: null }),
    time: () => null,
    checkTime: () => ({ time: null, refused: 'not recorded' }),
    hold: () => null,
    checkHold: () => ({ held: null, refused: 'unavailable' }),
  });

  const harness = {
    drillGate: makeGate({
      stage: 'outside',
      outside: {
        kind: 'not-recorded',
        message: 'This time was not recorded.',
        detail: 'Collector paused during reload',
        span: { firstHour: H1936 + 220, lastHour: H1936 + 240 },
        pinned: true,
      },
    }, null),
    drillState: {
      breakdown: 'consumption' as string | null,
      spec: false,
      follow: false,
      hour: H1936 + 230 as number | null,
      country: holder,
    },
    onShow(breakdown: string, hour: number, country?: { tag: string; origin: number } | null) {
      shownCalls.push({ breakdown, hour, country });
    },
    onActivateRef(fn: (action: { hour: number; subject?: string }, activeHolder?: Holder | null) => void) {
      capturedActivate = fn;
    },
    onBreakdownMount(props: { playhead: number | null; hour: number | null; extent: readonly [number, number] }) {
      breakdownMounts++;
      breakdownProps = props;
    },
  };

  const host = globalThis as typeof globalThis & { __fuelBranchesHarness?: typeof harness };
  host.__fuelBranchesHarness = harness;

  const vite = await createServer({
    configFile: false,
    optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{
      name: 'fuel-branches-drill-test',
      enforce: 'pre',
      transform(code, id) {
        const norm = id.replaceAll('\\', '/');
        if (norm.endsWith('/widgets/fuel-branches/FuelBranchesWidget.tsx')) {
          return code
            .replace("from '../../charts/drill/useDrill'", "from 'fb-test-useDrill'")
            .replace("from '../../families/host/registry.tsx'", "from 'fb-test-familyWidget'")
            .replace("from '../fuel/FuelConsumptionBreakdown'", "from 'fb-test-breakdown'");
        }
        if (norm.endsWith('/host/DetailWindowHost.tsx')) {
          return code.replace("from '../../components/Popover'", "from 'fb-test-popover'");
        }
      },
      resolveId(id) {
        if (id.startsWith('fb-test-')) return `\0${id}`;
      },
      load(id) {
        if (id === '\0fb-test-popover') {
          return `import { createElement } from 'react';
            export const usePopover = () => ({ popup: { current: null }, openAt() {} });
            export const Popover = props => createElement('div', { 'data-popover': 'true', 'data-subject': props['data-subject'] }, props.children);`;
        }
        if (id === '\0fb-test-familyWidget') {
          return `import { createElement } from 'react';
            export const FamilyWidget = props => {
              globalThis.__fuelBranchesHarness.onActivateRef(props.onActivate);
              return createElement('div', { 'data-family': props.family });
            };`;
        }
        if (id === '\0fb-test-breakdown') {
          return `import { createElement } from 'react';
            export default function StubBreakdown(props) {
              globalThis.__fuelBranchesHarness.onBreakdownMount({ playhead: props.playhead, hour: props.hour, extent: props.extent });
              return createElement('div', { 'data-breakdown': 'mounted' });
            }`;
        }
        if (id === '\0fb-test-useDrill') {
          return `export const useDrill = (_runtime, parent) => {
            const h = globalThis.__fuelBranchesHarness;
            const open = h.drillState.breakdown !== null;
            return {
              state: h.drillState,
              saved: false,
              open,
              following: h.drillState.follow,
              parent,
              region: parent,
              gate: h.drillGate,
              hour: open ? h.drillGate.hour : null,
              outside: open ? h.drillGate.outside : null,
              time: () => null,
              hold: () => null,
              pinRefused: null,
              dispatch() {},
              followsAt: () => false,
              show: (breakdown, hour, country) => h.onShow(breakdown, hour, country),
              showAt: (breakdown, hour, country) => h.onShow(breakdown, hour, country),
              attach() {},
              pick() {},
              follow() {},
              pause() {},
              close() {},
            };
          };`;
        }
      },
    }],
  });

  try {
    const { default: FuelBranchesWidget } = await vite.ssrLoadModule('/ui/src/widgets/fuel-branches/FuelBranchesWidget.tsx') as typeof import('./FuelBranchesWidget');
    const makeWidget = (time: WidgetSpec['time']) => ({
      id: 'w1',
      kind: 'fuel-branches',
      configVersion: 1,
      title: 'Fuel by branch',
      trackIds: ['main'],
      countries: { kind: 'tags', tags: ['GER'] },
      time,
      options: {},
    }) as unknown as WidgetSpec;
    const runtime = {
      tracks: [TRACK],
      clock,
      client: {},
      requestSeek: () => true,
    } as never;

    const renderWith = (family: 'bars' | 'time-chart', time: WidgetSpec['time'] = { mode: 'follow' }) => renderToStaticMarkup(createElement(FuelBranchesWidget, {
      widget: makeWidget(time) as never,
      options: { decimals: 1 },
      runtime,
      active: true,
      mapOptions: {} as never,
      headerSlot: null,
      onDiagnostics: () => {},
      manifestDeclaration: { family, kind: 'fuel-branches', projection: {} as never },
    }));

    const gapHtml = renderWith('time-chart');
    assert.match(gapHtml, /Fuel consumption by consumer/, 'detail window stays open in a recording gap');
    assert.match(gapHtml, /Collector paused during reload/, 'gap refusal displays the recorded gap reason');
    assert.equal(breakdownMounts, 0, 'no fuel consumption breakdown mounts in a recording gap');
    assert.doesNotMatch(gapHtml, /aria-label="Follow playback"/, 'Follow is hidden while playback is paused in a gap');

    capturedActivate!({ hour: H1936 + 300, subject: keyId({ tag: 'slot:1', part: '2' }) }, holder);
    capturedActivate!({ hour: H1936 + 230, subject: `gap:0@${H1936 + 230}` }, holder);

    renderWith('bars');
    capturedActivate!({ hour: H1936 + 300, subject: '1' }, holder);

    assert.deepEqual(shownCalls, [
      { breakdown: 'consumption', hour: H1936 + 300, country: holder },
      { breakdown: 'consumption', hour: H1936 + 230, country: holder },
      { breakdown: 'consumption', hour: H1936 + 300, country: holder },
    ], 'both over-time (including gap clicks) and bars views pass the chart activation holder into the drill');

    harness.drillGate = makeGate({ stage: 'absent', reason: null }, null);
    const absentHtml = renderWith('bars');
    assert.match(absentHtml, /Fuel consumption by consumer/, 'detail window stays open when the holder is absent at the picked date');
    assert.match(absentHtml, /This country does not exist at this date\./);

    harness.drillGate = makeGate({
      stage: 'failed',
      failure: { message: 'Country identities could not be read.', retryable: true } as never,
      retry() {},
    }, null);
    const failedHtml = renderWith('time-chart');
    assert.match(failedHtml, /Fuel consumption by consumer/, 'detail window stays open on identity failure');
    assert.match(failedHtml, /Country identities could not be read\./);
    assert.match(failedHtml, /Retry/);

    clock.seek(H1936 + 150);
    harness.drillState = { breakdown: 'consumption', spec: false, follow: false, hour: H1936 + 300, country: holder };
    harness.drillGate = makeGate({ stage: 'open' }, 'DEU', H1936 + 300);
    const pinnedHtml = renderWith('bars');
    assert.match(pinnedHtml, /aria-label="Follow playback"/, 'paused seek from a gap into coverage exposes Follow');
    assert.deepEqual(
      breakdownProps,
      { playhead: H1936 + 300, hour: H1936 + 300, extent: [H1936, H1936 + 5000] },
      'pinned drill chart clips to the drill gated hour even when the following parent seeks earlier',
    );

    clock.seek(H1936 + 4000);
    harness.drillState = { breakdown: 'consumption', spec: false, follow: true, hour: null, country: holder };
    harness.drillGate = makeGate({ stage: 'open' }, 'DEU', H1936 + 4000);
    const rangeTime: WidgetSpec['time'] = { mode: 'range', firstOffset: 1000, lastOffset: 2000 };
    renderWith('time-chart', rangeTime);
    assert.deepEqual(
      breakdownProps,
      { playhead: H1936 + 4000, hour: null, extent: [H1936, H1936 + 5000] },
      'followed drill uses the full recording extent outside a parent range so history reaches hour 4000',
    );
    const beforeRangeActivate = shownCalls.length;
    capturedActivate!({ hour: H1936 + 4000, subject: '1' }, holder);
    assert.equal(shownCalls.length, beforeRangeActivate, 'chart activation still respects the parent range extent');
    const followedExtent = (breakdownProps as { extent: readonly [number, number] } | null)!.extent;
    const [start, end] = demandHistoryRange(followedExtent, H1936 + 4000, 720);
    assert.equal(end, H1936 + 4000);
    assert.ok(detailWindows(followedExtent, [start, end], H1936 + 4000, 720).includes(Math.floor((H1936 + 4000) / 720)));
  } finally {
    delete host.__fuelBranchesHarness;
    await vite.close();
  }
});
