import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { tokensFromValues } from '../../charts/tokens.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import { syntheticDashboardSource } from '../../dashboard/source.fixture.ts';
import type { useAir } from '../../dashboard/useAirSuperiority';

test('the air timeline key renders the actual gap and unknown patterns while retaining its other legends', async () => {
  const tokens = { ...tokensFromValues(() => '#556677', 'sans-serif'), error: '#ff0033', surface: '#112233', hatchSpacing: 12, gapOpacity: 0.16 };
  const source = syntheticDashboardSource('source', '1');
  const branch = source.metadata.branches[0];
  const state: ReturnType<typeof useAir> & { chartLegend: { id: string; label: string }[] } = {
    tokens, legend: [{ id: 'side', label: 'Synthetic coalition', color: '#445566' }],
    mode: { mode: 'sides', viewpoint: '', sides: [], regions: [], theatres: ['1'] },
    track: { id: 'main', roleId: 'primary', label: 'Main campaign', branchIndex: 0, anchorOffset: null,
      source, branch, axisOffset: 0, selectedTag: null },
    window: { firstHour: branch.firstHour, lastHour: branch.lastHour },
    data: { view: 'timeline', branchId: branch.id, available: true, firstHour: branch.firstHour, lastHour: branch.lastHour,
      days: [], sides: [], segments: {}, anchorReads: {}, rows: [{ region: 1, name: 'Synthetic region', cells: [] }],
      excluded: [], theatreReads: [], scale: 1, dayHours: 24, badBelow: 0, goodAbove: 1, warnings: [] },
    projectedTimeline: { rows: [{ id: '1', label: 'Synthetic region', cells: [] }], periods: [], viewAt: () => undefined },
    labelCatalogue: ['Synthetic region'], extent: [branch.firstHour, branch.lastHour],
    error: null, needsViewpoint: false, retry() {}, beforeBranch: false, playhead: branch.lastHour, hour: branch.lastHour,
    readHour: branch.firstHour, projectedMoment: [], tooltip: () => null, hoverRevision: '', choices: undefined,
    country: tag => ({ name: tag, flag: undefined }), factionNames: new Map(), glides: false, glideStyle: {},
    hoursPerSecond: 0, timelineBehind: false, loading: [], chartLegend: [],
  };
  const globals = globalThis as typeof globalThis & { __airPattern?: typeof state };
  globals.__airPattern = state;
  const replacements: Record<string, string> = {
    '/dashboard/useAirSuperiority.ts': 'export const useAir = () => globalThis.__airPattern;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({open() {}, menu: null});',
    '/charts/heat/HeatChart.tsx': 'export default props => {globalThis.__airPattern.chartLegend = props.legend; return null;};',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'air-pattern-key', enforce: 'pre',
      transform: (_source, id) => Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1],
    }] });
  try {
    const Widget = (await vite.ssrLoadModule('/ui/src/widgets/air-superiority-timeline/AirSuperiorityTimelineWidget.tsx')).default;
    const props = { widget: { options: { look: 'margin' }, time: { mode: 'campaign' } }, runtime: {}, active: true } as unknown as WidgetProps;
    const html = renderToStaticMarkup(createElement(Widget, props));
    assert.match(html, /No activity/); assert.match(html, /Not recorded/); assert.match(html, /Unknown/);
    assert.match(html, /color-mix\(in srgb, #ff0033 16%, #112233\)/);
    assert.match(html, /background-size:12px 12px/);
    assert.match(html, /background-image:url/);
    assert.doesNotMatch(html, /Blank =|Shaded =|Hatched =/);
    assert.deepEqual(state.chartLegend.map(item => item.id), ['side', 'contested', 'filtered']);
    props.widget.options = { look: 'split' };
    renderToStaticMarkup(createElement(Widget, props));
    assert.deepEqual(state.chartLegend.map(item => item.id), ['side', 'filtered']);
  } finally { await vite.close(); delete globals.__airPattern; }
});
