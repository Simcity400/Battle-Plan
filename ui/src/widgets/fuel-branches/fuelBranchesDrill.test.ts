import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { countryDrillTagAt } from '../../dashboard/countryIdentity.ts';
import { datedCountrySpans } from '../../dashboard/datedCountry.ts';
import { keyId } from '../../dashboard/keyedSeriesData.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { setOption } from '../../dashboard/options.ts';
import { PlaybackController } from '../../dashboard/playback.ts';
import { holderSelection, type Gate } from '../../dashboard/regionGate.ts';
import { axisAt, FOLLOW, regionOutside, recordedHour } from '../../dashboard/regionTime.ts';
import { followSpan, watchBoundary } from '../../dashboard/widgetTime.ts';
import { hitTestTimeChart } from '../../families/time-chart/hitTest.ts';
import { parseTimeChartHit } from '../../families/time-chart/hostAdapter.ts';
import { layoutTimeChart } from '../../families/time-chart/layout.ts';
import type { TimeChartModel } from '../../families/time-chart/model.ts';
import type { CountryIdentityHistory } from '../../generated/dashboard';
import { demandHistoryRange } from '../fuel/fuelConsumption.ts';
import { detailWindows } from '../fuel/fuelDrill.ts';
import { FAMILY_VIEW_UI, familyViewOptionFields } from '../manifest.ts';
import { OPTIONS as FUEL_BRANCHES_OPTIONS } from './definition.ts';
import {
  actionCountryHolder,
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
    lastHour: H1936 + 500,
    gaps: [{ firstHour: H1936 + 220, lastHour: H1936 + 240, reason: 'Collector paused during reload' }],
  },
} as unknown as BoundTrack;

test('fuel-branches registers a view UI wrapper and declares the reached country option for both bars and over-time views', () => {
  const entry = FAMILY_VIEW_UI['fuel-branches'];
  assert.ok(entry, 'fuel-branches must be registered in FAMILY_VIEW_UI');
  assert.equal(entry.views, undefined, 'wrapper applies to both bars and time views');
  const token = JSON.stringify({ campaign: 'camp', branch: 'b0', tag: 'GER', originOffset: 50 });
  assert.ok(setOption(FUEL_BRANCHES_OPTIONS, {}, 'country', token), 'definition accepts the persisted country holder option');
  for (const view of ['default', 'time']) {
    const fields = familyViewOptionFields('fuel-branches', view);
    const written = setOption(fields, {}, 'country', token);
    assert.ok(written, `${view} view persists a chosen holder through the country option`);
  }
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
  assert.deepEqual(actionCountryHolder({ countryHolder: originalHolder }), originalHolder);
  assert.equal(actionCountryHolder(undefined), null);
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
  clock.setBounds({ firstHour: H1936, lastHour: H1936 + 500 });
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

test('FuelBranchesWidget keeps the detail window mounted across refusals, uses the chart activation holder, keeps pinned playhead independent of parent seeks, and updates Follow', async () => {
  const clock = new PlaybackController();
  clock.setBounds({ firstHour: H1936, lastHour: H1936 + 500 });
  clock.seek(H1936 + 230);

  const holder = { tag: 'GER', origin: H1936 + 50 };
  const shownCalls: Array<{ breakdown: string; hour: number; country: { tag: string; origin: number } | null | undefined }> = [];
  let capturedActivate: ((action: { hour: number; subject?: string; meta?: Readonly<Record<string, unknown>> }) => void) | undefined;
  let breakdownProps: { playhead: number | null; hour: number | null } | null = null;
  let breakdownMounts = 0;

  const makeGate = (stage: Gate['stage'], tag: string | null, gatedHour = H1936 + 300): Gate => ({
    region: { widget: {} as WidgetSpec, action: FOLLOW },
    track: TRACK,
    active: true,
    hour: stage.stage === 'open' ? gatedHour as never : null,
    lastRecorded: (H1936 + 500) as never,
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
      history: { key: 'slot:1', spans: [{ firstHour: H1936, lastHour: H1936 + 500 }] },
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
    onActivateRef(fn: (action: { hour: number; subject?: string; meta?: Readonly<Record<string, unknown>> }) => void) {
      capturedActivate = fn;
    },
    onBreakdownMount(props: { playhead: number | null; hour: number | null }) {
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
            .replace("from '../fuel/FuelConsumptionBreakdown'", "from 'fb-test-breakdown'")
            .replace("from '../../components/ContextMenu'", "from 'fb-test-contextMenu'");
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
        if (id === '\0fb-test-contextMenu') {
          return `export const useContextMenu = () => ({ open() {}, menu: null });`;
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
              globalThis.__fuelBranchesHarness.onBreakdownMount({ playhead: props.playhead, hour: props.hour });
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
    const widget = {
      id: 'w1',
      kind: 'fuel-branches',
      configVersion: 1,
      title: 'Fuel by branch',
      trackIds: ['main'],
      countries: { kind: 'tags', tags: ['GER'] },
      time: { mode: 'follow' },
      options: {},
    } as unknown as WidgetSpec;
    const runtime = {
      tracks: [TRACK],
      clock,
      client: {},
      requestSeek: () => true,
    } as never;

    const renderWithFamily = (family: 'bars' | 'time-chart') => renderToStaticMarkup(createElement(FuelBranchesWidget, {
      widget: widget as never,
      options: { decimals: 1 },
      runtime,
      active: true,
      mapOptions: {} as never,
      headerSlot: null,
      onDiagnostics: () => {},
      manifestDeclaration: { family, kind: 'fuel-branches', projection: {} as never },
    }));

    const gapHtml = renderWithFamily('time-chart');
    assert.match(gapHtml, /Fuel consumption by consumer/, 'detail window stays open in a recording gap');
    assert.match(gapHtml, /Collector paused during reload/, 'gap refusal displays the recorded gap reason');
    assert.equal(breakdownMounts, 0, 'no fuel consumption breakdown mounts in a recording gap');
    assert.doesNotMatch(gapHtml, /aria-label="Follow playback"/, 'Follow is hidden while playback is paused in a gap');

    capturedActivate!({ hour: H1936 + 300, subject: keyId({ tag: 'slot:1', part: '2' }), meta: { countryHolder: holder } });
    capturedActivate!({ hour: H1936 + 230, subject: `gap:0@${H1936 + 230}`, meta: { countryHolder: holder } });

    renderWithFamily('bars');
    capturedActivate!({ hour: H1936 + 300, subject: '1', meta: { countryHolder: holder } });

    assert.deepEqual(shownCalls, [
      { breakdown: 'consumption', hour: H1936 + 300, country: holder },
      { breakdown: 'consumption', hour: H1936 + 230, country: holder },
      { breakdown: 'consumption', hour: H1936 + 300, country: holder },
    ], 'both over-time (including gap clicks) and bars views pass the chart activation holder into the drill');

    harness.drillGate = makeGate({ stage: 'absent', reason: null }, null);
    const absentHtml = renderWithFamily('bars');
    assert.match(absentHtml, /Fuel consumption by consumer/, 'detail window stays open when the holder is absent at the picked date');
    assert.match(absentHtml, /This country does not exist at this date\./);

    harness.drillGate = makeGate({
      stage: 'failed',
      failure: { message: 'Country identities could not be read.', retryable: true } as never,
      retry() {},
    }, null);
    const failedHtml = renderWithFamily('time-chart');
    assert.match(failedHtml, /Fuel consumption by consumer/, 'detail window stays open on identity failure');
    assert.match(failedHtml, /Country identities could not be read\./);
    assert.match(failedHtml, /Retry/);

    clock.seek(H1936 + 150);
    harness.drillState = { breakdown: 'consumption', spec: false, follow: false, hour: H1936 + 300, country: holder };
    harness.drillGate = makeGate({ stage: 'open' }, 'DEU', H1936 + 300);
    const pinnedHtml = renderWithFamily('bars');
    assert.match(pinnedHtml, /aria-label="Follow playback"/, 'paused seek from a gap into coverage exposes Follow');
    assert.deepEqual(
      breakdownProps,
      { playhead: H1936 + 300, hour: H1936 + 300 },
      'pinned drill chart clips to the drill gated hour even when the following parent seeks earlier',
    );
  } finally {
    delete host.__fuelBranchesHarness;
    await vite.close();
  }
});
