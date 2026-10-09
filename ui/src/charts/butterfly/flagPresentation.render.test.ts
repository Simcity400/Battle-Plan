import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { tokensFromValues } from '../tokens.ts';
import type { FrameTask } from '../scheduler';
import type { ButterflyItem, ButterflyProps } from './ButterflyChart';
import { SECONDARY_DASH } from '../palette.ts';

// The actual component owns fitting, decoding, keyboard selection and publication.
test('country Butterfly tooltips remain bounded after flag status expands, with full speech and totals', async () => {
  const bars: number[][] = [], strokes: number[][] = [];
  let dash: number[] = [];
  const context = new Proxy({ measureText: (text: string) => ({ width: text.length * 7 }),
    roundRect: (x: number, y: number, width: number, height: number) => { bars.push([x, y, width, height]); },
    setLineDash: (value: number[]) => { dash = [...value]; }, stroke: () => { strokes.push(dash); } },
    { get: (target, key) => Reflect.get(target, key) ?? (() => {}) });
  const canvas = { clientWidth: 600, clientHeight: 360, width: 0, height: 0, style: {},
    getContext: () => context, getBoundingClientRect: () => ({ left: 0, top: 0, width: 600, height: 360 }) };
  const state = { canvas, container: { clientHeight: 360, scrollTop: 0 }, effects: [] as (() => unknown)[],
    task: undefined as FrameTask | undefined, key: undefined as ((event: { key: string; shiftKey?: boolean; preventDefault(): void }) => void) | undefined,
    click: undefined as ((event: { clientX: number; clientY: number }) => void) | undefined };
  const host = globalThis as typeof globalThis & { __butterflyFlags?: typeof state; __butterflyFlagEffect?: (effect: () => unknown) => void };
  host.__butterflyFlags = state; host.__butterflyFlagEffect = effect => { state.effects.push(effect); };
  const before = { Image: globalThis.Image, ResizeObserver: globalThis.ResizeObserver, window: globalThis.window };
  const images: { complete: boolean; naturalWidth: number; onerror?: () => void }[] = [];
  globalThis.Image = class { complete = false; naturalWidth = 0; src = ''; onerror?: () => void;
    constructor() { images.push(this); } } as unknown as typeof Image;
  globalThis.ResizeObserver = class { observe() {} disconnect() {} } as unknown as typeof ResizeObserver;
  globalThis.window = { devicePixelRatio: 1 } as Window & typeof globalThis;
  const tokens = tokensFromValues(() => '#123456', 'sans-serif', undefined, { motion: 'smooth', reducedMotion: true, countryColours: 'raw' });
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'butterfly-flag-browser-surface', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/charts/useChartTokens.ts')) return `export const useChartTokens = () => (${JSON.stringify(tokens)}); export const useViewportHeight = () => 500;`;
        if (path.endsWith('/components/ContextMenu.tsx')) return 'export const useContextMenu = () => ({ menu: null, open() {} });';
        if (path.endsWith('/charts/scheduler.ts')) return `export const scheduler = { add(task) { globalThis.__butterflyFlags.task = task; return () => {}; }, wake() {}, setVisible() {} };`;
        if (path.endsWith('/charts/butterfly/ButterflyChart.tsx')) return source
          .replace('canvas = useRef<HTMLCanvasElement>(null)', 'canvas = useRef(globalThis.__butterflyFlags.canvas)')
          .replace('root = useRef<HTMLDivElement>(null)', 'root = useRef(globalThis.__butterflyFlags.container)')
          .replace('onKeyDown={event => {', 'onKeyDown={globalThis.__butterflyFlags.key = event => {')
          .replace('onClick={event => {', 'onClick={globalThis.__butterflyFlags.click = event => {')
          .replace(/\buseEffect\(/g, 'globalThis.__butterflyFlagEffect(');
      },
    }] });
  let tooltip: Pick<typeof import('../tooltip.ts'),
    'tooltipStore' | 'tooltipHeight' | 'tooltipRoom' | 'showTooltip' | 'hideTooltip'> | undefined;
  const cleanups: (() => void)[] = [];
  try {
    const { default: ButterflyChart } = await vite.ssrLoadModule('/ui/src/charts/butterfly/ButterflyChart.tsx');
    const { tooltipStore, tooltipHeight, tooltipRoom, showTooltip, hideTooltip } = await vite.ssrLoadModule('/ui/src/charts/tooltip.ts');
    tooltip = { tooltipStore, tooltipHeight, tooltipRoom, showTooltip, hideTooltip };
    const rows = Array.from({ length: 5000 }, (_, index) => ({ label: `Native member ${index}`,
      countryFlag: { flag: `synthetic-butterfly-member-${index}`, flagPending: false } }));
    const picked: { item: ButterflyItem; side: 'left' | 'right' }[] = [];
    const props: ButterflyProps<ButterflyItem> = { label: 'Synthetic losses', active: true, leftLabel: 'Taken', rightLabel: 'Dealt', ratioLabel: 'Ratio',
      items: [{ id: 'native', label: 'Native country', countryFlag: { flag: 'synthetic-butterfly-native' }, color: '#123456',
        left: -1, right: 2, leftText: '−1', rightText: '2', ratioText: '−2', segment: 'known' }],
      onSelect: (item, side) => { picked.push({ item, side }); },
      tooltip: () => ({ title: 'Native country', rows: [...rows, { label: 'Total', value: '9007199254740993', strong: true }] }) };
    renderToStaticMarkup(createElement(ButterflyChart, props));
    for (const effect of state.effects) { const cleanup = effect(); if (typeof cleanup === 'function') cleanups.push(cleanup as () => void); }
    state.task!.frame({ now: 0, dt: 0 });
    assert.equal(bars.length, 2, 'both signed sides retain magnitude geometry');
    assert.ok(bars.every(bar => bar[2] > 0)); assert.deepEqual(strokes, [[...SECONDARY_DASH]], 'only the negative bar has a dashed outline');
    const centre = (bar: number[]) => ({ clientX: bar[0] + bar[2] / 2, clientY: bar[1] + bar[3] / 2 });
    state.click!(centre(bars[0])); state.click!(centre(bars[1]));
    state.click!({ clientX: 0, clientY: 0 });
    assert.deepEqual(picked.map(pick => pick.side), ['left', 'right'], 'only drawn rows activate their clicked side');
    assert.equal(picked[0].item, props.items[0]); assert.equal(picked[0].item.leftText, '−1');
    state.key!({ key: 'Home', preventDefault() {} });
    state.task!.frame({ now: 16, dt: 16 });
    state.key!({ key: 'Enter', preventDefault() {} }); state.key!({ key: 'Enter', shiftKey: true, preventDefault() {} });
    assert.deepEqual(picked.map(pick => pick.side), ['left', 'right', 'left', 'right']);
    const shown = tooltip!.tooltipStore.get()!;
    assert.ok(tooltip!.tooltipHeight(shown.content) <= tooltip!.tooltipRoom(500), 'pending flag text must be fitted after decoding');
    assert.equal(shown.content.rows.at(-1)?.value, '9007199254740993');
    assert.match(shown.content.spoken!, /Native member 4999/);
    assert.ok(images.length > 0 && images.length < 100, 'hidden full speech rows never start decoders');
    const count = images.length;
    for (const image of images) { image.complete = true; image.onerror!(); }
    state.task!.frame({ now: 32, dt: 16 });
    const failed = tooltip!.tooltipStore.get()!.content;
    assert.ok(tooltip!.tooltipHeight(failed) <= tooltip!.tooltipRoom(500), 'expanded decode refusal text must stay in the window');
    assert.equal(failed.rows.at(-1)?.value, '9007199254740993');
    assert.match(failed.spoken!, /Native member 4999/);
    assert.equal(images.length, count);
    tooltip!.showTooltip('other-chart', { title: 'Other pointer', rows: [] }, 8, 9);
    const held = tooltip!.tooltipStore.get();
    state.task!.frame({ now: 48, dt: 16 });
    assert.equal(tooltip!.tooltipStore.get(), held, 'background keyboard state cannot reclaim another chart');
    state.key!({ key: 'Home', preventDefault() {} }); state.task!.frame({ now: 64, dt: 16 });
    assert.notEqual(tooltip!.tooltipStore.get()?.owner, 'other-chart');
  } finally {
    for (const cleanup of cleanups.reverse()) cleanup(); tooltip?.hideTooltip('other-chart');
    globalThis.Image = before.Image; globalThis.ResizeObserver = before.ResizeObserver; globalThis.window = before.window;
    delete host.__butterflyFlags; delete host.__butterflyFlagEffect; await vite.close();
  }
});
