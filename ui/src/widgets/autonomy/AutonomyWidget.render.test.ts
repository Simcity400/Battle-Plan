import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { TimelineProps } from '../../charts/timeline/Timeline';
import type { ChartTokens } from '../../charts/tokens';
import { validatePreparedAutonomy } from './preparedAutonomy.ts';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';

test('autonomy marks bootstrap bands and empty sampled or pending results before numeric geometry arrives', async () => {
  const span = { firstHour: 100, lastHour: 105 };
  const band = { ...span, startsSegment: true, level: 0, overlord: null, lowest: true, highest: true };
  const reply = { branchId: '1', scoreScale: 100000, levels: [{ id: 'mod-level', name: 'Mod level', defined: true }],
    subjects: [], bands: [], summaries: {}, exactHours: [], recorded: [span], unidentified: [], warnings: [],
    identities: [{ tag: 'A', identities: [{ ...span, slot: 1, countryId: 10 }] }],
    coverage: { firstHour: 100, lastHour: 99, complete: false } };
  const host = globalThis as typeof globalThis & {
    __autonomyData?: ReturnType<typeof validatePreparedAutonomy>;
    __autonomyTimeline?: TimelineProps;
    __autonomyGeometryReady?: boolean;
  };
  const replacements: Record<string, string> = {
    '/dashboard/preparedSeries.ts': 'export const usePreparedWindow = () => ({ data: globalThis.__autonomyData, geometryReady: globalThis.__autonomyGeometryReady, retry() {}, onSummaryView() {}, navigate() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => {};',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({});',
    '/dashboard/countryLooks.ts': 'export const useCountryLooks = () => ({ name: tag => tag, flag() {}, revision: 0 });',
    '/dashboard/useHoverLabels.ts': 'export const useHoverLabels = () => ({ ask() {}, labelAt: () => tag => ({ name: tag }), revision: 0 });',
    '/dashboard/parityRows.ts': 'export const useParityRows = () => {};',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ surface: "#000000", text: "#ffffff", accent: "#ff0000", lineWidth: { series: 2 } });',
    '/charts/timeline/Timeline.tsx': 'export default props => { globalThis.__autonomyTimeline = props; return null; };',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'autonomy-first-view-fixture', enforce: 'pre',
      transform(_source, id) {
        const path = id.replaceAll('\\', '/');
        return Object.entries(replacements).find(([suffix]) => path.endsWith(suffix))?.[1];
      },
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/autonomy/AutonomyWidget.tsx');
    const props = { widget: { id: 'autonomy', kind: 'autonomy', trackIds: ['main'], countries: { kind: 'all' }, time: { mode: 'campaign' }, options: {} },
      runtime: { client: {}, tracks: [{ id: 'main', axisOffset: 0, branch: { id: '1', ...span },
        source: { id: 'source', revision: '1', metadata: { countries: [] } } }],
      clock: { getHour: () => 105, getSnapshot: () => ({ hoursPerSecond: 0 }) } }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = (data: unknown, geometryReady = true) => {
      host.__autonomyData = validatePreparedAutonomy(data);
      host.__autonomyGeometryReady = geometryReady;
      delete host.__autonomyTimeline;
      return renderToStaticMarkup(createElement(Widget, props));
    };
    const empty = render({ ...reply, approximate: [span] });
    assert.match(empty, /Subject data is still being prepared/);
    assert.match(empty, /Approximate/);
    assert.doesNotMatch(empty, /No subjects found|was not recorded/);
    const pending = render({ ...reply, recorded: [], pending: [span] });
    assert.match(pending, /Still being prepared/);
    assert.doesNotMatch(pending, /No subjects found|was not recorded/);
    const reached = render({ ...reply, recorded: [{ firstHour: 105, lastHour: 105 }], exactHours: [105] });
    assert.match(reached, /Subject data is still being prepared/);
    assert.doesNotMatch(reached, /No subjects found|was not recorded/);
    const partial = render({ ...reply, coverage: { firstHour: 100, lastHour: 104, complete: false } });
    assert.match(partial, /Subject data is still being prepared/);
    const preparedRange = render({ ...reply, coverage: { ...span, complete: false } });
    assert.match(preparedRange, /No subjects found for these countries/);
    const preparedPoint = render({ ...reply, coverage: { ...span, complete: true },
      recorded: [{ firstHour: 105, lastHour: 105 }], exactHours: [105] }, false);
    assert.match(preparedPoint, /Subject data is still being prepared/);
    assert.doesNotMatch(preparedPoint, /No subjects found/);
    const exact = render({ ...reply, coverage: { ...span, complete: true }, approximate: [] });
    assert.match(exact, /No subjects found for these countries/);
    assert.doesNotMatch(exact, /Approximate|still being prepared/);

    const drawn = render({ ...reply, bands: [{ tag: 'A', runs: [band] }], approximate: [span] });
    assert.match(drawn, /Approximate/);
    const timeline = host.__autonomyTimeline!;
    assert.equal(timeline.lanes.length, 1);
    assert.deepEqual(timeline.approximate, [span]);
    const alpha: number[] = [];
    const stack: number[] = [];
    const context = new Proxy({ globalAlpha: 1 }, { get(target, key) {
      if (key === 'save') return () => stack.push(target.globalAlpha);
      if (key === 'restore') return () => { target.globalAlpha = stack.pop()!; };
      if (key === 'fillRect') return () => alpha.push(target.globalAlpha);
      return key in target ? target[key as keyof typeof target] : () => {};
    } }) as unknown as CanvasRenderingContext2D;
    timeline.lanes[0].paint!(context, { top: 0, height: 54, left: 0, right: 600, hours: [100, 106],
      toX: hour => (hour - 100) * 100, playhead: null },
    { surface: '#000000', text: '#ffffff', accent: '#ff0000', lineWidth: { series: 2 } } as ChartTokens);
    assert.deepEqual(alpha, [0.55], 'categorical bands are faded even without a numeric summary');
  } finally {
    delete host.__autonomyData;
    delete host.__autonomyTimeline;
    delete host.__autonomyGeometryReady;
    await vite.close();
  }
});

test('a group counted at hours still being prepared is loading, not countries with no subjects', async () => {
  const span = { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour };
  // ENG is a subject throughout; GER, the human player, never is.
  const data = validatePreparedAutonomy({ branchId: BRANCH.id, scoreScale: 100000, levels: [{ id: 'mod-level', name: 'Mod level', defined: true }],
    subjects: [], summaries: {}, exactHours: [BRANCH.lastHour], recorded: [span], unidentified: [], warnings: [], approximate: [],
    bands: [{ tag: 'ENG', runs: [{ ...span, startsSegment: true, level: 0, overlord: null, lowest: true, highest: true }] }],
    identities: ['GER', 'ENG'].map((tag, index) => ({ tag, identities: [{ ...span, slot: index + 1, countryId: index + 101 }] })),
    coverage: { ...span, complete: true } });
  const state = { data, groups: pendingGroups() };
  const host = globalThis as typeof globalThis & { __autonomyPending?: typeof state }; host.__autonomyPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/autonomy/AutonomyWidget.tsx', {
    '/dashboard/preparedSeries.ts': 'export const usePreparedWindow = () => ({ data: globalThis.__autonomyPending.data, geometryReady: true, retry() {}, onSummaryView() {}, navigate() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => {};',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__autonomyPending.groups, retry() {} });',
    '/dashboard/countryLooks.ts': 'export const useCountryLooks = () => ({ name: tag => tag, flag() {}, revision: 0 });',
    '/dashboard/useHoverLabels.ts': 'export const useHoverLabels = () => ({ ask() {}, labelAt: () => tag => ({ name: tag }), revision: 0 });',
    '/dashboard/parityRows.ts': 'export const useParityRows = () => {};',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ surface: "#000000", text: "#ffffff", accent: "#ff0000", lineWidth: { series: 2 } });',
    '/charts/timeline/Timeline.tsx': 'export default () => null;',
  });
  try {
    const props = { widget: { id: 'autonomy', kind: 'autonomy', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: {} },
      runtime: { client: {}, tracks: [{ id: 'main', axisOffset: 0, branch: BRANCH, source: { id: 'source', revision: '1', metadata: { countries: [] } } }],
        clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ hoursPerSecond: 0 }) } }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // The human players ever, and so each hour's, are not known while hours are still being prepared.
    const pending = render();
    assert.match(pending, /aria-label="This range is still being prepared\."/);
    assert.doesNotMatch(pending, /No subjects found/);
    // Read, GER is the only human player and never a subject.
    state.groups = pendingGroups({ final: true, humans: [{ tag: 'GER', firstHour: 0, lastHour: 200 }] });
    assert.match(render(), /No subjects found for these countries\./);
  } finally { delete host.__autonomyPending; await close(); }
});
