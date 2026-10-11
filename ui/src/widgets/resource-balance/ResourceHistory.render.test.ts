import assert from 'node:assert/strict';
import test from 'node:test';
import { isValidElement } from 'react';
import type { ComponentProps, ReactElement, ReactNode } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type TimeChart from '../../charts/TimeChart';
import type { ResourceBalanceReply } from '../../generated/dashboard';
import { balanceOf, resourceName } from './balanceData.ts';
import { formatSigned } from '../../charts/valueAxis.ts';

function elements(node: ReactNode): ReactElement<Record<string, unknown>>[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!isValidElement<Record<string, unknown>>(node)) return [];
  return [node, ...elements(node.props.children as ReactNode)];
}

test('resource chip keys and values match every drawn declaration, including repeated colours and gaps', async () => {
  const state = { chart: undefined as ComponentProps<typeof TimeChart> | undefined,
    strokes: [] as { color: string; dash: readonly number[]; runs: { value: number }[] }[], fills: 0, saved: [] as [string, string][] };
  const host = globalThis as typeof globalThis & { __resourceKeys?: typeof state };
  host.__resourceKeys = state;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'resource-curve-keys', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/charts/TimeChart.tsx')) return 'export default props => { globalThis.__resourceKeys.chart = props; return null; };';
        if (path.endsWith('/charts/draw.ts')) return source.replace('export function drawLine(', 'function unusedDrawLine(')
          .replace('export function drawGapFill(', 'function unusedDrawGapFill(')
          + '\nexport const drawLine = (_ctx, _frame, _tokens, runs, color, options) => globalThis.__resourceKeys.strokes.push({ color, runs, dash: options.dash ?? [] });'
          + '\nexport const drawGapFill = () => { globalThis.__resourceKeys.fills++; };';
        if (path.endsWith('/widgets/resource-balance/ResourceBalanceWidget.tsx')) return source
          .replace(/\buseMemo(?:<[^;\n]*>)?\(/g, '((factory) => factory())(')
          .replace(/\buseState(?:<[^\n]*>)?\(/g, '((initial) => [initial, () => {}])(')
          .replace(/\buseEffect\(/g, '(() => {})(') + '\nexport { OverTime };';
      },
    }] });
  try {
    const { OverTime } = await vite.ssrLoadModule('/ui/src/widgets/resource-balance/ResourceBalanceWidget.tsx');
    const data: ResourceBalanceReply = { branchId: 'b', tag: 'C', decimals: 5, warnings: [], trades: [], dealHours: [],
      resources: Array.from({ length: 5 }, (_, id) => ({ id, token: id === 4 ? 'r0' : `r${id}`, ordinal: id === 4 ? 2 : 1,
        name: `Resource ${id}`, icon: null })),
      series: Array.from({ length: 5 }, (_, id) => ({ id, runs: [{ firstHour: 100, lastHour: 110, startsSegment: true,
        producedRaw: String((id + 1) * 100000), importedRaw: '0', subjectsRaw: '0', toExportRaw: '0', exportedRaw: '0',
        availableRaw: String((id + 1) * 100000), afterProjectsRaw: String((id + 1) * 100000), productionRaw: String(id * 200000) }] })) };
    const props = { data, numbers: data, tokens: { categories: ['#123456', '#abcdef'] }, viewport: 1000,
      options: { resources: 'r1', gap: true }, extent: [100, 110], playhead: 105, active: true,
      runtime: { clock: { getSnapshot: () => ({ hoursPerSecond: 0 }) } },
      track: { source: { id: 's', revision: '1' }, branch: { id: 'b', firstHour: 100 } },
      partnerAt: () => (tag: string) => ({ name: tag }), onPartners() {}, onMenu() {}, onPick() {},
      setOption: (key: string, value: string) => state.saved.push([key, value]) };
    for (const chosen of ['r1', 'r1 r2', 'r0', 'r0 r1 r2 r3']) {
      props.options.resources = chosen;
      const node = OverTime(props);
      const html = renderToStaticMarkup(node);
      const buttons = elements(node).filter(element => element.type === 'button');
      const selected = buttons.filter(button => button.props['aria-pressed'] === true);
      const resources = data.resources.filter(resource => chosen.split(' ').includes(resource.token));
      assert.equal(selected.length, resources.length, 'each declaration from the old legend remains represented');
      assert.doesNotMatch(html, /chart-legend|every further|Solid: supply/);
      assert.match(html, /Supply above demand in each key/);
      state.strokes = []; state.fills = 0;
      for (const layer of state.chart!.layers!) layer.draw?.({} as never, {} as never, {} as never, {} as never);
      assert.equal(state.fills, resources.length === 1 ? 1 : 0, 'shading requires exactly one resolved declaration, including duplicate tokens');
      for (const [index, button] of selected.entries()) {
        const resource = resources[index];
        const run = data.series.find(series => series.id === resource.id)!.runs[0];
        const value = formatSigned(balanceOf(run).net, data.decimals);
        assert.equal(button.props.type, 'button', 'Enter and Space use a native button');
        assert.equal(button.props['aria-label'], `${resourceName(resource)}, net ${value} per day`);
        const keyLines = elements(button.props.children as ReactNode).filter(element => element.type === 'line');
        assert.equal(keyLines.length, 2);
        const supplyIndex = state.strokes.findIndex((stroke, index) => index % 2 === 0 && stroke.runs[0].value === resource.id + 1);
        assert.ok(supplyIndex >= 0);
        for (const [part, keyLine] of keyLines.entries()) {
          const stroke = state.strokes[supplyIndex + part];
          assert.equal(keyLine.props.stroke, stroke.color);
          assert.equal(keyLine.props.strokeDasharray, stroke.dash.join(' '));
        }
        assert.match(renderToStaticMarkup(button), new RegExp(value.replace(/[+]/g, '\\+')));
      }
      if (chosen === 'r1') {
        assert.equal(selected[0].props['aria-disabled'], true, 'the only resource cannot be cleared');
        (selected[0].props.onClick as () => void)();
        assert.deepEqual(state.saved, []);
        (buttons.find(button => button.props['aria-label'] === 'Resource 2')!.props.onClick as () => void)();
        assert.deepEqual(state.saved.pop(), ['resources', 'r1 r2']);
      }
      if (chosen === 'r0 r1 r2 r3') {
        assert.ok(state.strokes.some(stroke => stroke.dash.length === 2), 'a repeated palette uses a primary dash');
        (selected.at(-1)!.props.onClick as () => void)();
        assert.deepEqual(state.saved.pop(), ['resources', 'r1 r2 r3'], 'a duplicate declaration still toggles the whole token');
      }
    }
    props.playhead = 111;
    props.options.resources = 'r1';
    const missing = renderToStaticMarkup(OverTime(props));
    assert.match(missing, /Resource 1, net Not recorded per day/);
    assert.doesNotMatch(missing, /Resource 1, net 0/);
  } finally { await vite.close(); delete host.__resourceKeys; }
});
