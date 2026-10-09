import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { tokensFromValues } from '../tokens.ts';
import type { FrameTask } from '../scheduler';
import type { ChordChartProps } from './ChordChart';

// Run the actual draw/publish callbacks; only the browser surfaces and scheduler are synthetic.
test('a folded-country keyboard tooltip decodes displayed flags only and cannot reclaim another chart on settlement', async () => {
  const handlers = new Map<string, (event: unknown) => void>();
  const context = new Proxy({ measureText: (text: string) => ({ width: text.length * 7 }) },
    { get: (target, key) => Reflect.get(target, key) ?? (() => {}) });
  const canvas = { clientWidth: 600, clientHeight: 360, width: 0, height: 0,
    getContext: () => context, getBoundingClientRect: () => ({ left: 0, top: 0, width: 600, height: 360 }),
    addEventListener: (name: string, handler: (event: unknown) => void) => handlers.set(name, handler),
    removeEventListener: (name: string) => handlers.delete(name) };
  const state = { canvas, effects: [] as (() => unknown)[], task: undefined as FrameTask | undefined, wakes: 0,
    latest: undefined as { current: { props: ChordChartProps } } | undefined };
  const host = globalThis as typeof globalThis & { __chordFlags?: typeof state; __chordFlagEffect?: (effect: () => unknown) => void };
  host.__chordFlagEffect = effect => { state.effects.push(effect); };
  const before = { Image: globalThis.Image, ResizeObserver: globalThis.ResizeObserver, window: globalThis.window };
  const images: { src: string; complete: boolean; naturalWidth: number; naturalHeight: number; onload?: () => void; onerror?: () => void }[] = [];
  host.__chordFlags = state;
  globalThis.Image = class { complete = false; naturalWidth = 0; naturalHeight = 0; src = ''; onload?: () => void; onerror?: () => void;
    constructor() { images.push(this); } } as unknown as typeof Image;
  globalThis.ResizeObserver = class { observe() {} disconnect() {} } as unknown as typeof ResizeObserver;
  globalThis.window = { devicePixelRatio: 1 } as Window & typeof globalThis;
  const tokens = tokensFromValues(() => '#123456', 'sans-serif', undefined, { motion: 'smooth', reducedMotion: true, countryColours: 'raw' });
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'chord-flag-browser-surface', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/charts/useChartTokens.ts')) return `export const useChartTokens = () => (${JSON.stringify(tokens)}); export const useViewportHeight = () => 500;`;
        if (path.endsWith('/charts/scheduler.ts')) return `export const scheduler = { add(task) { globalThis.__chordFlags.task = task; return () => {}; },
          wake() { globalThis.__chordFlags.wakes++; }, setVisible() {} };`;
        if (path.endsWith('/charts/chord/ChordChart.tsx')) return source
          .replace('canvas = useRef<HTMLCanvasElement>(null)', 'canvas = useRef(globalThis.__chordFlags.canvas)')
          .replace('const latest = useRef({ props, tokens, viewportHeight });', 'const latest = useRef({ props, tokens, viewportHeight }); globalThis.__chordFlags.latest = latest;')
          .replace(/\buseEffect\(/g, 'globalThis.__chordFlagEffect(');
      },
    }] });
  let tooltip: Pick<typeof import('../tooltip.ts'),
    'tooltipStore' | 'tooltipHeight' | 'tooltipRoom' | 'showTooltip' | 'hideTooltip'> | undefined;
  const cleanups: (() => void)[] = [];
  try {
    const { default: ChordChart } = await vite.ssrLoadModule('/ui/src/charts/chord/ChordChart.tsx');
    const { tooltipStore, tooltipHeight, tooltipRoom, showTooltip, hideTooltip } = await vite.ssrLoadModule('/ui/src/charts/tooltip.ts');
    tooltip = { tooltipStore, tooltipHeight, tooltipRoom, showTooltip, hideTooltip };
    const rows = Array.from({ length: 5000 }, (_, index) => ({ label: `Native member ${index}`,
      countryFlag: { flag: `synthetic-visible-chord-flag-${index}`, flagPending: false } }));
    const props: ChordChartProps = { label: 'Synthetic membership', caption: '', active: true, onError: message => assert.equal(message, null),
      frame: { data: { nodes: [{ id: 'other', label: 'Other countries', color: '#123456' },
        { id: 'peer', label: 'Native peer', color: '#654321' }], flows: [{ id: 'flow', source: 'other', target: 'peer', value: 1 }] } },
      tooltip: () => ({ title: 'Other countries', rows: [...rows, { label: 'Total', value: '9007199254740993', strong: true }] }) };
    renderToStaticMarkup(createElement(ChordChart, props));
    for (const effect of state.effects) { const cleanup = effect(); if (typeof cleanup === 'function') cleanups.push(cleanup as () => void); }
    state.task!.frame({ now: 0, dt: 0 });
    const key = (value: string) => handlers.get('keydown')!({ key: value, preventDefault() {} });
    key('Home');
    const shown = tooltip!.tooltipStore.get()!;
    assert.ok(shown.content.rows.length < 100);
    assert.ok(tooltip!.tooltipHeight(shown.content) <= tooltip!.tooltipRoom(500), 'pending row text is fitted after decode');
    assert.equal(shown.content.rows.at(-1)?.value, '9007199254740993');
    assert.match(shown.content.spoken!, /Native member 4999/);
    assert.ok(images.length > 0 && images.length < 100, 'hidden full keyboard rows must never start flag decoders');
    tooltip!.showTooltip('other-chart', { title: 'Pointer on another chart', rows: [] }, 8, 9);
    const held = tooltip!.tooltipStore.get();
    const count = images.length;
    images[0].complete = true; images[0].onerror!();
    assert.ok(state.wakes > 0);
    state.task!.frame({ now: 16, dt: 16 });
    assert.equal(tooltip!.tooltipStore.get(), held, 'a decoder wake cannot take the shared tooltip back');
    assert.equal(images.length, count, 'settlement must not restart hidden/evicted decoders');
    key('Home');
    assert.notEqual(tooltip!.tooltipStore.get()?.owner, 'other-chart', 'a new user navigation key takes ownership');
    for (const image of images) { image.complete = true; image.onerror!(); }
    state.task!.frame({ now: 24, dt: 8 });
    const failed = tooltip!.tooltipStore.get()!.content;
    assert.ok(tooltip!.tooltipHeight(failed) <= tooltip!.tooltipRoom(500), 'failure status expansion stays within the window');
    assert.equal(failed.rows.at(-1)?.value, '9007199254740993');
    assert.match(failed.spoken!, /Native member 4999/);
    assert.equal(images.length, count, 'the final refit never decodes hidden rows');
    const omitted = failed.rows.filter(row => row.omittedRows != null);
    assert.equal(omitted.length, 1);
    assert.equal(omitted[0].omittedRows! + failed.rows.filter(row => row.label.startsWith('Native member ')).length, 5000);
    const recoveredRows = rows.map((row, index) => index === 0
      ? { ...row, countryFlag: { ...row.countryFlag, flag: 'synthetic-recovered-visible-flag' } } : row);
    state.latest!.current.props = { ...props,
      tooltip: () => ({ title: 'Other countries', rows: [...recoveredRows, { label: 'Total', value: '9007199254740993', strong: true }] }) };
    key('Home');
    const recovered = images.at(-1)!;
    assert.equal(images.length, count + 1, 'the new visible flag starts one replacement decoder');
    assert.equal(recovered.src, 'synthetic-recovered-visible-flag');
    tooltip!.showTooltip('other-chart', { title: 'Pointer stays on another chart', rows: [] }, 8, 9);
    const recoveryOwner = tooltip!.tooltipStore.get();
    recovered.complete = true; recovered.naturalWidth = 24; recovered.naturalHeight = 16; recovered.onload!();
    state.task!.frame({ now: 32, dt: 16 });
    assert.equal(tooltip!.tooltipStore.get(), recoveryOwner, 'success also cannot reclaim a background keyboard tooltip');
    key('Home');
    assert.equal(tooltip!.tooltipStore.get()?.content.rows[0].countryFlag?.flag, 'synthetic-recovered-visible-flag');
    const datedHeader = { flagReason: 'Unavailable at the selected date', flagPending: false };
    state.latest!.current.props = { ...props, frame: { data: { ...props.frame.data!, nodes: props.frame.data!.nodes.map(node =>
      node.id === 'peer' ? { ...node, countryFlag: { flag: 'synthetic-later-peer-flag' } } : node) } },
      tooltip: () => ({ title: 'Dated peer', countryFlag: datedHeader, rows, scrollable: true, scrollId: 'peer' }) };
    const beforeScroll = images.length;
    key('End');
    const scrolling = tooltip!.tooltipStore.get()!;
    assert.equal(scrolling.content.rows, rows, 'scrolling breakdowns keep every row');
    assert.equal(scrolling.content.countryFlag, datedHeader, 'dated metadata wins over the drawn node');
    assert.equal(images.length, beforeScroll, 'only the tooltip layer decodes scrolling row flags');
    const { measureTooltipScroll } = await vite.ssrLoadModule('/ui/src/charts/tooltip.ts');
    measureTooltipScroll(scrolling.owner, 120);
    key('PageDown');
    assert.equal(tooltip!.tooltipStore.get()?.scrollTop, 120);
    handlers.get('wheel')!({ deltaY: 1, deltaMode: 2, preventDefault() {} });
    assert.equal(tooltip!.tooltipStore.get()?.scrollTop, 240);
    tooltip!.showTooltip('other-chart', { title: 'Another scrolling chart', rows, scrollable: true }, 8, 9);
    const otherScroll = tooltip!.tooltipStore.get();
    key('PageDown');
    assert.equal(tooltip!.tooltipStore.get(), otherScroll, 'background page keys cannot scroll another owner');

  } finally {
    for (const cleanup of cleanups.reverse()) cleanup();
    tooltip?.hideTooltip('other-chart');
    globalThis.Image = before.Image; globalThis.ResizeObserver = before.ResizeObserver; globalThis.window = before.window;
    delete host.__chordFlags; delete host.__chordFlagEffect; await vite.close();
  }
});
