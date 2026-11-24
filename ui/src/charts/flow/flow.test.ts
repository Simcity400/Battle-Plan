import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { FlowDiagramProps } from './FlowDiagram';
import type { FrameTask } from '../scheduler';
import { tokensFromValues } from '../tokens.ts';
import { flowPlaybackSnap, FlowMotion, flowStops, labelAt, linkAt, nodeAt, ribbonTop, slotOrder } from './layout.ts';
import type { FlowGeometry, FlowLink, FlowNode } from './layout';

const node = (id: string, side: 'source' | 'target', extra: Partial<FlowNode> = {}): FlowNode => ({ id, side, label: id, color: '#888', order: id, ...extra });
const link = (source: string, target: string, value: number, order = ''): FlowLink => ({ id: `${source}>${target}${order}`, source, target, value, color: '#123', order });
const geometry: FlowGeometry = { left: 100, right: 300, top: 0, bottom: 114, nodeWidth: 10, gap: 10 };
const nodes = [node('A', 'source'), node('B', 'source'), node('X', 'target'), node('Y', 'target')];

function styleOf() {
  const values = new Map<string, string>();
  return { getPropertyValue: (name: string) => values.get(name) ?? '', setProperty: (name: string, value: string) => { values.set(name, value); } };
}

test('slots put the largest first, ties by key and a last node at the end', () => {
  const sizes = new Map([['A', 1], ['B', 5], ['C', 5], ['O', 99]]);
  const order = slotOrder([node('C', 'source'), node('O', 'source', { last: true }), node('A', 'source'), node('B', 'source'), node('X', 'target')], sizes, 'source');
  assert.deepEqual(order.map(item => item.id), ['B', 'C', 'A', 'O']);
});


test('a campaign scale keeps widths as amounts while the total changes', () => {
  const motion = new FlowMotion();
  const frame = motion.step({ nodes, links: [link('A', 'X', 25)] }, { ...geometry, scale: 100, slots: 2 }, 16, 0, true);
  assert.equal(frame.links[0].width, 25);
});



test('a snap jumps across a gap instead of gliding', () => {
  const motion = new FlowMotion();
  motion.step({ nodes, links: [link('A', 'X', 40)] }, geometry, 16, 100, true);
  const frame = motion.step({ nodes, links: [link('B', 'Y', 10)] }, geometry, 16, 100, true);
  assert.deepEqual(frame.links.map(item => item.id), ['B>Y']);
  assert.equal(frame.settled, true);
});

test('backward playback or a campaign-to-Live cutoff removes future geometry while forward playback keeps easing', () => {
  const motion = new FlowMotion();
  motion.step({ nodes, links: [link('A', 'X', 40), link('B', 'Y', 60)] }, geometry, 16, 100, true);
  assert.equal(flowPlaybackSnap(10, 20), false);
  const forward = motion.step({ nodes, links: [link('A', 'X', 50)] }, geometry, 16, 100, flowPlaybackSnap(10, 20));
  assert.ok(forward.links.some(shape => shape.id === 'B>Y'), 'forward changes retain existing exit motion');
  assert.equal(flowPlaybackSnap(20, 10), true);
  const backward = motion.step({ nodes, links: [link('A', 'X', 10)] }, geometry, 16, 100, flowPlaybackSnap(20, 10));
  assert.deepEqual(backward.links.map(shape => shape.id), ['A>X']);
  assert.equal(backward.nodes.some(box => box.id === 'B' || box.id === 'Y'), false);
  assert.equal(backward.settled, true);
  assert.equal(flowPlaybackSnap(100, 10), true, 'whole-campaign extent to an earlier Live playhead');
  assert.equal(flowPlaybackSnap(undefined, 10), false, 'omitted extent preserves existing consumers');
  assert.equal(flowPlaybackSnap(10, undefined), false);
  assert.equal(flowPlaybackSnap(10, 10), false);
});

test('the point on a ribbon or a node finds it, and a thin ribbon is still easy to point at', () => {
  const shape = { x0: 0, x1: 100, y0: 0, y1: 50 };
  assert.ok(Math.abs(ribbonTop(shape, 0)!) < 1e-6);
  assert.equal(Math.round(ribbonTop(shape, 50)! * 1000) / 1000, 25);
  assert.ok(Math.abs(ribbonTop(shape, 100)! - 50) < 1e-6);
  assert.equal(ribbonTop(shape, 101), null);
  const motion = new FlowMotion();
  const frame = motion.step({ nodes, links: [link('A', 'X', 99.9), link('B', 'Y', 0.1)] }, geometry, 16, 0, true);
  const middle = (id: string) => { const item = frame.links.find(each => each.id === id)!; return ribbonTop(item, 200)! + item.width / 2; };
  assert.equal(linkAt(frame.links, 200, middle('A>X'))?.id, 'A>X');
  assert.equal(linkAt(frame.links, 200, middle('B>Y') + 2)?.id, 'B>Y');
  assert.equal(linkAt(frame.links, 50, 10), null);
  assert.equal(nodeAt(frame.nodes, 95, 5)?.id, 'A');
  assert.equal(nodeAt(frame.nodes, 50, 5), null);
});

test('the keyboard stops at each source node and its ribbons, then each target node', () => {
  const motion = new FlowMotion();
  const frame = motion.step({ nodes, links: [link('A', 'Y', 30), link('A', 'X', 10), link('B', 'X', 60)] }, geometry, 16, 0, true);
  assert.deepEqual(flowStops(frame).map(stop => `${stop.kind}:${stop.id}`),
    ['node:B', 'link:B>X', 'node:A', 'link:A>X', 'link:A>Y', 'node:X', 'node:Y']);
});


test('a backward drawn cutoff clears a vanished keyed tooltip with a stable callback and keeps full scroll rows decode-free', async () => {
  const context = new Proxy({ measureText: (text: string) => ({ width: text.length * 7 }) },
    { get: (target, key) => Reflect.get(target, key) ?? (() => {}) });
  const handlers = new Map<string, (event: unknown) => void>();
  const canvas = { clientWidth: 600, clientHeight: 360, offsetWidth: 600, width: 0, height: 0, style: styleOf(),
    getContext: () => context, getBoundingClientRect: () => ({ left: 0, top: 0, width: 600, height: 360 }),
    addEventListener(name: string, handler: (event: unknown) => void) { handlers.set(name, handler); },
    removeEventListener(name: string) { handlers.delete(name); } };
  const state = { canvas, effects: [] as (() => unknown)[], task: undefined as FrameTask | undefined,
    latest: undefined as { current: FlowDiagramProps } | undefined,
    key: undefined as ((event: { key: string; preventDefault(): void }) => void) | undefined, spoken: '' };
  const host = globalThis as typeof globalThis & { __flowCutoff?: typeof state; __flowEffect?: (effect: () => unknown) => void };
  host.__flowCutoff = state; host.__flowEffect = effect => { state.effects.push(effect); };
  const before = { Image: globalThis.Image, ResizeObserver: globalThis.ResizeObserver, window: globalThis.window, innerHeight: globalThis.innerHeight };
  let decoders = 0;
  globalThis.Image = class { complete = false; naturalWidth = 0; src = ''; constructor() { decoders++; } } as unknown as typeof Image;
  globalThis.ResizeObserver = class { observe() {} disconnect() {} } as unknown as typeof ResizeObserver;
  globalThis.window = { devicePixelRatio: 1, innerHeight: 500 } as Window & typeof globalThis; globalThis.innerHeight = 500;
  const tokens = tokensFromValues(() => '#123456', 'sans-serif', undefined, { motion: 'smooth', reducedMotion: false, countryColours: 'raw' });
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'flow-cutoff-browser-surface', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/charts/tokens.ts')) return `export const readTokens = () => (${JSON.stringify(tokens)}); export const subscribeTokens = () => () => {};`;
        if (path.endsWith('/charts/scheduler.ts')) return `export const scheduler = { add(task) { globalThis.__flowCutoff.task = task; return () => {}; }, wake() {}, setVisible() {} };`;
        if (path.endsWith('/charts/flow/FlowDiagram.tsx')) return source
          .replace('canvas = useRef<HTMLCanvasElement>(null)', 'canvas = useRef(globalThis.__flowCutoff.canvas)')
          .replace('const latest = useRef(props);', 'const latest = useRef(props); globalThis.__flowCutoff.latest = latest;')
          .replace("const [spoken, setSpoken] = useState('');", "const spoken = ''; const setSpoken = text => { globalThis.__flowCutoff.spoken = text; };")
          .replace('onKeyDown={keyDown}', 'onKeyDown={(globalThis.__flowCutoff.key = keyDown)}')
          .replace(/\buseEffect\(/g, 'globalThis.__flowEffect(');
      },
    }] });
  let tooltip: Pick<typeof import('../tooltip.ts'), 'tooltipStore' | 'measureTooltipScroll'> | undefined;
  const cleanups: (() => void)[] = [];
  try {
    const { default: FlowDiagram } = await vite.ssrLoadModule('/ui/src/charts/flow/FlowDiagram.tsx');
    const { tooltipStore, measureTooltipScroll } = await vite.ssrLoadModule('/ui/src/charts/tooltip.ts');
    tooltip = { tooltipStore, measureTooltipScroll };
    const rows = Array.from({ length: 5000 }, (_, index) => ({ label: `Native occurrence ${index}`,
      countryFlag: { flag: `synthetic-flow-hidden-${index}` } }));
    const props: FlowDiagramProps = { label: 'Synthetic transfers', active: true, stretch: 'same', playbackHour: 20, hour: 20, nodes,
      links: [{ ...link('A', 'X', 1), id: 'earlier' }, { ...link('B', 'Y', 2), id: 'future' }],
      tooltip: hit => ({ title: `${hit.kind}:${hit.kind === 'node' ? hit.node.id : hit.link.id}`,
        countryFlag: { flag: 'synthetic-flow-header' }, rows, scrollable: true, scrollId: 'stable' }) };
    renderToStaticMarkup(createElement(FlowDiagram, props));
    for (const effect of state.effects) { const cleanup = effect(); if (typeof cleanup === 'function') cleanups.push(cleanup as () => void); }
    state.task!.frame({ now: 0, dt: 0 });
    const key = (value: string) => state.key!({ key: value, preventDefault() {} });
    key('Home'); key('ArrowDown');
    state.task!.frame({ now: 8, dt: 8 });
    await Promise.resolve();
    assert.equal(tooltip!.tooltipStore.get()?.content.title, 'link:future');
    assert.equal(tooltip!.tooltipStore.get()?.content.rows, rows);
    assert.match(state.spoken, /Native occurrence 4999/);
    assert.equal(decoders, 1, 'only the displayed header may start a canvas decoder, never the full scroll inventory');
    const shown = tooltip!.tooltipStore.get()!;
    tooltip!.measureTooltipScroll(shown.owner, 300);
    let prevented = false;
    handlers.get('wheel')!({ deltaY: 1, deltaMode: 2, preventDefault() { prevented = true; } });
    assert.equal(tooltip!.tooltipStore.get()?.scrollTop, 300, 'page wheels use the measured tooltip viewport');
    assert.equal(prevented, true);
    const callback = state.latest!.current.tooltip;
    state.latest!.current = { ...props, playbackHour: 10, hour: 10,
      nodes: nodes.filter(n => n.id === 'A' || n.id === 'X'), links: [{ ...link('A', 'X', 1), id: 'earlier' }] };
    assert.equal(state.latest!.current.tooltip, callback);
    state.task!.frame({ now: 16, dt: 8 });
    await Promise.resolve();
    assert.equal(tooltip!.tooltipStore.get(), null, 'the vanished future stop cannot leave an old tooltip behind');
    assert.equal(state.spoken, '');
    assert.equal(decoders, 1);
  } finally {
    for (const cleanup of cleanups.reverse()) cleanup();
    globalThis.Image = before.Image; globalThis.ResizeObserver = before.ResizeObserver; globalThis.window = before.window; globalThis.innerHeight = before.innerHeight;
    delete host.__flowCutoff; delete host.__flowEffect; await vite.close();
  }
});

test('a drawn flag or name opens its node only where the diagram asks, and a bar keeps its own pick', async () => {
  const context = new Proxy({ measureText: (text: string) => ({ width: text.length * 7 }) },
    { get: (target, key) => Reflect.get(target, key) ?? (() => {}) });
  const handlers = new Map<string, (event: unknown) => void>();
  const canvas = { clientWidth: 600, clientHeight: 360, offsetWidth: 600, width: 0, height: 0, style: styleOf(),
    getContext: () => context, getBoundingClientRect: () => ({ left: 0, top: 0, width: 600, height: 360 }),
    addEventListener(name: string, handler: (event: unknown) => void) { handlers.set(name, handler); },
    removeEventListener(name: string) { handlers.delete(name); } };
  const state = { canvas, effects: [] as (() => unknown)[], task: undefined as FrameTask | undefined,
    latest: undefined as { current: FlowDiagramProps } | undefined };
  const host = globalThis as typeof globalThis & { __flowLabels?: typeof state; __flowLabelEffect?: (effect: () => unknown) => void };
  host.__flowLabels = state; host.__flowLabelEffect = effect => { state.effects.push(effect); };
  const before = { ResizeObserver: globalThis.ResizeObserver, window: globalThis.window };
  globalThis.ResizeObserver = class { observe() {} disconnect() {} } as unknown as typeof ResizeObserver;
  globalThis.window = { devicePixelRatio: 1, innerHeight: 500 } as Window & typeof globalThis;
  const tokens = tokensFromValues(() => '#123456', 'sans-serif', undefined, { motion: 'smooth', reducedMotion: true, countryColours: 'raw' });
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'flow-label-surface', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/charts/tokens.ts')) return `export const readTokens = () => (${JSON.stringify(tokens)}); export const subscribeTokens = () => () => {};`;
        if (path.endsWith('/charts/scheduler.ts')) return `export const scheduler = { add(task) { globalThis.__flowLabels.task = task; return () => {}; }, wake() {}, setVisible() {} };`;
        if (path.endsWith('/charts/flow/FlowDiagram.tsx')) return source
          .replace('canvas = useRef<HTMLCanvasElement>(null)', 'canvas = useRef(globalThis.__flowLabels.canvas)')
          .replace('const latest = useRef(props);', 'const latest = useRef(props); globalThis.__flowLabels.latest = latest;')
          .replace(/\buseEffect\(/g, 'globalThis.__flowLabelEffect(');
      },
    }] });
  const cleanups: (() => void)[] = [];
  try {
    const { default: FlowDiagram } = await vite.ssrLoadModule('/ui/src/charts/flow/FlowDiagram.tsx');
    const picks: string[] = [];
    const labelPicks = new Map<string, [number, number]>();
    const props: FlowDiagramProps = { label: 'Synthetic flows', active: true, stretch: 'same', hour: 20, nodes,
      links: [link('A', 'X', 1), link('B', 'Y', 2)],
      onPick: hit => { picks.push(hit.kind === 'node' ? hit.node.id : hit.link.id); },
      onLabelPick: (node, hour) => { assert.equal(hour, 20); if (!labelPicks.has(node.id)) labelPicks.set(node.id, [at.x, at.y]); } };
    const at = { x: 0, y: 0 };
    renderToStaticMarkup(createElement(FlowDiagram, props));
    for (const effect of state.effects) { const cleanup = effect(); if (typeof cleanup === 'function') cleanups.push(cleanup as () => void); }
    state.task!.frame({ now: 0, dt: 0 });
    const click = (x: number, y: number) => { at.x = x; at.y = y; handlers.get('click')!({ button: 0, clientX: x, clientY: y }); };
    for (let x = 0; x <= 600; x += 3) for (let y = 0; y <= 360; y += 3) {
      const before = picks.length + labelPicks.size;
      click(x, y);
      assert.ok(picks.length + labelPicks.size - before <= 1, 'one click opens one thing');
    }
    assert.deepEqual([...labelPicks.keys()].sort(), ['A', 'B', 'X', 'Y'], 'every drawn country name or flag is a place to click');
    assert.ok(picks.includes('A') && picks.includes('X'), 'node bars keep opening their own breakdown');
    state.latest!.current = { ...props, onLabelPick: undefined };
    const [x, y] = labelPicks.get('A')!;
    const count = picks.length;
    click(x, y);
    assert.equal(picks.length, count);
  } finally {
    for (const cleanup of cleanups.reverse()) cleanup();
    globalThis.ResizeObserver = before.ResizeObserver; globalThis.window = before.window;
    delete host.__flowLabels; delete host.__flowLabelEffect; await vite.close();
  }
});

test('a label box holds the points of its flag and name only', () => {
  const box = { id: 'A', node: node('A', 'source'), x: 10, y: 20, width: 40, height: 16, alpha: 1 };
  assert.equal(labelAt([box], 10, 20)?.id, 'A');
  assert.equal(labelAt([box], 50, 36)?.id, 'A');
  assert.equal(labelAt([box], 51, 30), null);
  assert.equal(labelAt([{ ...box, alpha: 0.01 }], 20, 25), null, 'a vanishing label is not pointed at');
});
