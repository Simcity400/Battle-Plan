import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { tokensFromValues } from './tokens.ts';
import type { FrameTask } from './scheduler';
import type { ButterflyItem, ButterflyProps } from './butterfly/ButterflyChart';

test(`butterfly interaction follows position-only moves without size notifications`, async () => {
  let left = 30, top = 40;
  const bars: number[][] = [];
  const handlers = new Map<string, (event: any) => void>();
  const context = new Proxy({ measureText: (text: string) => ({ width: text.length * 7 }),
    roundRect: (x: number, y: number, width: number, height: number) => { bars.push([x, y, width, height]); } },
  { get: (target, key) => Reflect.get(target, key) ?? (() => {}) });
  const canvas = {
    clientWidth: 500, clientHeight: 300,
    width: 0, height: 0, style: {}, parentElement: {}, getContext: () => context,
    getBoundingClientRect: () => ({ left, top, width: 500, height: 300 }),
    addEventListener: (name: string, handler: (event: any) => void) => handlers.set(name, handler),
    removeEventListener: (name: string) => handlers.delete(name),
  };
  const fixture = { canvas, container: { clientHeight: 300, scrollTop: 0, addEventListener() {}, removeEventListener() {} }, effects: [] as (() => unknown)[],
    task: undefined as FrameTask | undefined, key: undefined as ((event: any) => void) | undefined,
    move: undefined as ((event: any) => void) | undefined, click: undefined as ((event: any) => void) | undefined };
  const host = globalThis as typeof globalThis & { __positionFixture?: typeof fixture; __positionEffect?: (effect: () => unknown) => void };
  host.__positionFixture = fixture; host.__positionEffect = effect => { fixture.effects.push(effect); };
  const originals = ['ResizeObserver', 'window'].map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const);
  let notifications = 0;
  Object.defineProperty(globalThis, 'ResizeObserver', { configurable: true,
    value: class { constructor(_notify: () => void) {} observe() {} disconnect() {} } });
  Object.defineProperty(globalThis, 'window', { configurable: true, value: { devicePixelRatio: 2,
    addEventListener() { notifications++; }, removeEventListener() {} } });
  const tokens = tokensFromValues(() => '#123456', 'sans-serif', undefined, { motion: 'smooth', reducedMotion: true, countryColours: 'raw' });
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'position-only-surface', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/charts/useChartTokens.ts')) return `export const useChartTokens = () => (${JSON.stringify(tokens)}); export const useViewportHeight = () => 800;`;
        if (path.endsWith('/components/ContextMenu.tsx')) return 'export const useContextMenu = () => ({ menu: null, open() {} });';
        if (path.endsWith('/charts/scheduler.ts')) return `export const scheduler = { add(task) { globalThis.__positionFixture.task = task; return () => {}; }, wake() {}, setVisible() {} };`;
        if (path.endsWith('/butterfly/ButterflyChart.tsx')) return source
          .replace('canvas = useRef<HTMLCanvasElement>(null)', 'canvas = useRef(globalThis.__positionFixture.canvas)')
          .replace('root = useRef<HTMLDivElement>(null)', 'root = useRef(globalThis.__positionFixture.container)')
          .replace('onKeyDown={event => {', 'onKeyDown={globalThis.__positionFixture.key = event => {')
          .replace('onPointerMove={event => {', 'onPointerMove={globalThis.__positionFixture.move = event => {')
          .replace('onClick={event => {', 'onClick={globalThis.__positionFixture.click = event => {')
          .replace(/\buseEffect\(/g, 'globalThis.__positionEffect(')
          .replace('barPath(ctx, layout.axis, barTop, side * size, layout.barHeight);',
            'ctx.roundRect(layout.axis + Math.min(0, side * size), barTop, size, layout.barHeight, 0); barPath(ctx, layout.axis, barTop, side * size, layout.barHeight);');
      },
    }] });
  const cleanups: (() => void)[] = [];
  try {
    const { tooltipStore } = await vite.ssrLoadModule('/ui/src/charts/tooltip.ts') as typeof import('./tooltip.ts');
    const selected: string[] = [];
    const { default: Chart } = await vite.ssrLoadModule('/ui/src/charts/butterfly/ButterflyChart.tsx');
    const props: ButterflyProps<ButterflyItem> = { label: 'Synthetic butterfly', active: true, leftLabel: 'Left', rightLabel: 'Right', ratioLabel: 'Ratio',
      items: [{ id: 'a', label: 'A', color: '#123456', left: 1, right: 2, leftText: '1', rightText: '2', ratioText: '2', segment: 'known' }],
      tooltip: item => ({ title: item.label, rows: [] }), onSelect: item => { selected.push(item.id); } };
    renderToStaticMarkup(createElement(Chart, props));
    handlers.set('keydown', fixture.key!); handlers.set('pointermove', fixture.move!); handlers.set('click', fixture.click!);
    for (const effect of fixture.effects) { const cleanup = effect(); if (typeof cleanup === 'function') cleanups.push(cleanup as () => void); }
    const frame = () => fixture.task!.frame({ now: 0, dt: 0 });
    for (let i = 0; i < 3; i++) frame();
    let point = { clientX: left + bars[1][0] + bars[1][2] / 2, clientY: top + bars[1][1] + bars[1][3] / 2 };
    const move = () => { handlers.get('pointermove')!(point); frame(); };
    move();
    assert.equal(tooltipStore.get()?.content.title, 'A');
    const originalBacking = [canvas.width, canvas.height];
    const notificationsBefore = notifications;
    left += 200; top += 80; point = { clientX: point.clientX + 200, clientY: point.clientY + 80 };
    move();
    handlers.get('click')!({ ...point, button: 0 });
    assert.deepEqual(selected, ['a'], 'the click still reaches the drawn item');
    assert.equal(tooltipStore.get()?.content.title, 'A', 'hover agrees with the click after moving');
    assert.deepEqual([canvas.width, canvas.height], originalBacking, 'a translation retains the backing size');
    assert.equal(notifications, notificationsBefore, 'the move has no size, scroll or window notification');
    handlers.get('keydown')!({ key: 'Home', preventDefault() {} }); frame();
    const before = tooltipStore.get()!;
    left += 50; top += 25;
    handlers.get('keydown')!({ key: 'Home', preventDefault() {} }); frame();
    assert.equal(tooltipStore.get()!.x, before.x + 50, 'keyboard placement uses the current horizontal position');
    assert.equal(tooltipStore.get()!.y, before.y + 25, 'keyboard placement uses the current vertical position');
    handlers.get('pointerleave')?.({});
  } finally {
    for (const cleanup of cleanups.reverse()) cleanup();
    for (const [key, descriptor] of originals) { if (descriptor) Object.defineProperty(globalThis, key, descriptor); else Reflect.deleteProperty(globalThis, key); }
    delete host.__positionFixture; delete host.__positionEffect; await vite.close();
  }
});
