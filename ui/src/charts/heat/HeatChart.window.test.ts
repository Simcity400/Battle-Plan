import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { HeatChartProps } from './HeatChart';
import type { TimeChartProps } from '../TimeChart';
import { tokensFromValues } from '../tokens.ts';

test('campaign labels, scrolled cells and keyboard rows share the bounded viewport', async () => {
  const memory: unknown[] = [];
  const effects: (() => unknown)[] = [];
  let cursor = 0, measured = 0;
  const fixture = {
    useMemo<T>(factory: () => T) { return factory(); },
    useRef<T>(value: T) { const index = cursor++; return memory[index] ??= { current: value }; },
    useState<T>(value: T) { const index = cursor++; memory[index] ??= value;
      return [memory[index], (next: T | ((previous: T) => T)) => {
        memory[index] = typeof next === 'function' ? (next as (previous: T) => T)(memory[index] as T) : next;
      }]; },
    useLayoutEffect(effect: () => unknown) { effects.push(effect); },
  };
  const host = globalThis as typeof globalThis & { __heatHooks?: typeof fixture };
  host.__heatHooks = fixture;
  const originals = new Map(['document', 'ResizeObserver', 'HTMLCanvasElement'].map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)]));
  class Canvas {
    clientWidth = 800; clientHeight = 300;
    getBoundingClientRect() { return { left: 0, top: 0, width: 1600, height: this.clientHeight * 2 }; }
  }
  Object.defineProperty(globalThis, 'HTMLCanvasElement', { configurable: true, value: Canvas });
  Object.defineProperty(globalThis, 'ResizeObserver', { configurable: true, value: class { observe() {} disconnect() {} } });
  Object.defineProperty(globalThis, 'document', { configurable: true, value: { createElement: () => ({ getContext: () => ({
    font: '', measureText: (text: string) => { measured++; return { width: text.length * 7 }; },
  }) }) } });
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'heat-window-hooks', enforce: 'pre',
      transform(source, id) {
        if (id.replaceAll('\\', '/').endsWith('/heat/HeatChart.tsx')) return source
          .replace(/import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__heatHooks;')
          .replace("from '../useChartTokens'", "from 'heat-test-tokens'");
      },
      resolveId(id) { if (id === 'heat-test-tokens') return '\0heat-test-tokens'; },
      load(id) { if (id === '\0heat-test-tokens') return `export const useChartTokens = () => (${JSON.stringify(tokensFromValues(() => '#8899aa', 'sans-serif'))});`; },
    }] });
  try {
    const { default: HeatChart } = await vite.ssrLoadModule('/ui/src/charts/heat/HeatChart.tsx');
    const surface = { clientWidth: 800, clientHeight: 260, scrollTop: 0 };
    const labels = Array.from({ length: 5000 }, (_, index) => `Region ${index}`);
    labels[4999] = 'A translated region with a long campaign name';
    const reached: string[] = [];
    const props: HeatChartProps = { label: 'Heat', rows: labels.map((label, index) => ({ id: String(index), label,
      cells: [{ parts: [{ id: 'a', label: 'A', color: '#112233', share: 1 }] }] })), labelCatalogue: labels,
      periods: [{ firstHour: 0, lastHour: 23, readHour: 23 }], extent: [0, 23], branchStart: 0,
      playhead: null, look: 'split', active: true, revision: 1,
      tooltip: row => { reached.push(row.id); return null; } };
    type Element = { props: { children: Element | Element[]; ref?: { current: unknown }; style?: { height: number };
      onScroll: () => void; onKeyDownCapture: (event: { key: string }) => void;
      onPointerMoveCapture: (event: { target: Canvas; clientX: number; clientY: number }) => void } };
    const child = (element: Element) => Array.isArray(element.props.children) ? element.props.children[0] : element.props.children;
    let view: Element, chart: TimeChartProps, plot: Element;
    const render = async () => {
      cursor = 0; effects.length = 0;
      view = child(HeatChart(props)); view.props.ref!.current = surface;
      plot = child(child(child(view)));
      chart = child(plot).props as unknown as TimeChartProps;
      for (const effect of effects) effect();
      await Promise.resolve();
    };
    await render(); await render(); await render();
    assert.equal(chart!.rows!.labelWidth, labels[4999].length * 7 + 12);
    assert.ok(chart!.rows!.labelWidth > 118);
    assert.ok(chart!.rows!.labels.length < 50);
    assert.ok(plot!.props.style!.height < 400);
    const width = chart!.rows!.labelWidth, initialMeasurements = measured;
    props.playhead = 23;
    props.rows = props.rows.slice(0, 10); await render(); await render();
    assert.equal(chart!.rows!.labelWidth, width, 'playback inventory cannot shrink the campaign gutter');
    assert.equal(measured, initialMeasurements, 'unchanged labels do not repeat text measurement');
    props.rows = labels.map((label, index) => ({ id: String(index), label, cells: [{ parts: [] }] }));
    surface.scrollTop = 2000 * 13 + 5; view!.props.onScroll(); await render();
    const canvas = new Canvas();
    view!.props.onPointerMoveCapture({ target: canvas, clientX: 600, clientY: (22 + 6.5) * 2 });
    chart!.rows!.tooltip(0, 23);
    assert.equal(reached.at(-1), '2000', 'Size-scaled pointer resolves the partially visible logical row');
    view!.props.onKeyDownCapture({ key: 'ArrowDown' }); await render();
    chart!.rows!.tooltip(0, 23);
    assert.equal(reached.at(-1), '2001');
    for (let index = 2001; index < 4999; index++) view!.props.onKeyDownCapture({ key: 'ArrowDown' });
    await render(); chart!.rows!.tooltip(0, 23);
    assert.equal(reached.at(-1), '4999', 'keyboard reaches the entire logical inventory');
    assert.ok(surface.scrollTop > 60_000);
    assert.ok(plot!.props.style!.height < 400);
    assert.ok(chart!.rows!.labels.length < 50);
    view!.props.onKeyDownCapture({ key: 'ArrowUp' }); await render(); chart!.rows!.tooltip(0, 23);
    assert.equal(reached.at(-1), '4998');
  } finally {
    delete host.__heatHooks;
    for (const [key, descriptor] of originals) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor);
      else Reflect.deleteProperty(globalThis, key);
    }
    await vite.close();
  }
});
