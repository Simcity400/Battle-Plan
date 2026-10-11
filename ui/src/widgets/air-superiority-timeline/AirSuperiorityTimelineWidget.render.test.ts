import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { tokensFromValues } from '../../charts/tokens.ts';
import type { WidgetProps } from '../../dashboard/definitions';

test('the air timeline key renders the actual gap and unknown patterns while retaining its other legends', async () => {
  const tokens = { ...tokensFromValues(() => '#556677', 'sans-serif'), error: '#ff0033', surface: '#112233', hatchSpacing: 12, gapOpacity: 0.16 };
  const state = { tokens, legend: [{ id: 'side', label: 'Synthetic coalition', color: '#445566' }],
    mode: { theatres: [1] }, track: { branch: { firstHour: 0 } }, window: {}, data: { available: true },
    projectedTimeline: { rows: [{ id: '1', label: 'Synthetic region' }], periods: [] }, labelCatalogue: new Map(), extent: [0, 100],
    tooltip() {}, chartLegend: [] as { id: string; label: string }[] };
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
