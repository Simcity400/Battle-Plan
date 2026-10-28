import assert from 'node:assert/strict';
import test from 'node:test';
import type { ReactElement } from 'react';
import { createServer } from 'vite';
import type { FrameTask } from '../scheduler';
import { tokensFromValues } from '../tokens.ts';
import type { RaceChartProps } from './RaceChart';
import { raceHeight } from './race.ts';

test('actual remote race keeps global scrolling, keys, scale and owned asynchronous focus for 5,000 rows', async () => {
  const slots: unknown[] = [], effects = new Map<number, readonly unknown[] | undefined>();
  const pending: (() => unknown)[] = [], listeners = new Map<string, (event: unknown) => void>();
  let cursor = 0, task: FrameTask | undefined;
  const fixture = {
    ranges: [] as [number, number][], scales: [] as [number, number][], tip: null as { title: string } | null,
    useId: () => 'remote-race',
    useRef(initial: unknown) { const i = cursor++; return slots[i] ??= { current: initial }; },
    useState(initial: unknown) { const i = cursor++; slots[i] ??= initial; return [slots[i], (value: unknown) => { slots[i] = value; }]; },
    useEffect(run: () => unknown, deps?: readonly unknown[]) {
      const i = cursor++, before = effects.get(i);
      if (!effects.has(i) || !deps || !before || deps.some((v, at) => !Object.is(v, before[at]))) pending.push(run);
      effects.set(i, deps);
    },
    scheduler: { add(next: FrameTask) { task = next; return () => {}; }, wake() {}, setVisible() {} },
  };
  const host = globalThis as typeof globalThis & { __remoteRace?: typeof fixture };
  host.__remoteRace = fixture;
  const tokens = tokensFromValues(() => '#123456', 'sans-serif', undefined, { motion: 'smooth', reducedMotion: true, countryColours: 'raw' });
  const context = new Proxy({ measureText: (text: string) => ({ width: text.length * 7 }) }, { get: (value, key) => Reflect.get(value, key) ?? (() => {}) });
  const element = { width: 0, height: 0, clientWidth: 1000, clientHeight: 300, getContext: () => context,
    getBoundingClientRect: () => ({ left: 0, top: 0, width: 1000, height: 300 }),
    addEventListener: (name: string, callback: (event: unknown) => void) => listeners.set(name, callback), removeEventListener() {} };
  const box = { scrollTop: 0 };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'remote-race-fixture', enforce: 'pre', transform(source, id) {
      const path = id.replaceAll('\\', '/');
      if (path.endsWith('/charts/race/RaceChart.tsx')) return source.replace("import { useEffect, useId, useRef, useState } from 'react';", 'const { useEffect, useId, useRef, useState } = globalThis.__remoteRace;');
      if (path.endsWith('/charts/tokens.ts')) return `export const readTokens = () => (${JSON.stringify(tokens)}); export const subscribeTokens = () => () => {};`;
      if (path.endsWith('/charts/scheduler.ts')) return 'export const scheduler = globalThis.__remoteRace.scheduler;';
      if (path.endsWith('/charts/surface.ts')) return `export const toLocal = (x,y) => ({x,y}); export const watchCanvasSize = () => ({read:()=>({layoutWidth:1000,layoutHeight:300,backing:{width:1000,height:300,scaleX:1,scaleY:1}}),disconnect(){}});`;
      if (path.endsWith('/charts/race/draw.ts')) return `export const FLAG={x:8,width:21,height:14},NAME_X=36,VALUE_GAP=8,MIN_LABEL=100,NAME_PAD=8,VALUE_PAD=8;
        export const drawDateOverlay=()=>{}; export const drawRaceBar=(_ctx,layout)=>layout.left;
        export const drawRaceGrid=(_ctx,_layout,_tokens,min,max)=>globalThis.__remoteRace.scales.push([min,max]);`;
      if (path.endsWith('/charts/tooltip.ts')) return `export const tooltipStore={get:()=>null}; export const fitTooltip=forms=>forms[0];
        export const showTooltip=(_owner,content)=>{globalThis.__remoteRace.tip=content;}; export const hideTooltip=()=>{globalThis.__remoteRace.tip=null;}; export const tooltipText=content=>content.title;`;
    } }] });
  const cleanups: (() => unknown)[] = [];
  try {
    const { default: Chart } = await vite.ssrLoadModule('/ui/src/charts/race/RaceChart.tsx');
    const picks: string[] = [];
    const items = (first: number, count: number) => Array.from({ length: count }, (_, i) => ({ key: String(first + i), value: 1, label: '1', stretch: 0, segments: [{ id: 'total', value: 1, color: '#123456' }] }));
    const props: RaceChartProps = { label: 'Production', active: true, slots: 5000, items: items(0, 20), name: key => key,
      tooltip: key => ({ title: `Actor ${key}`, rows: [] }), onPick: key => { picks.push(key); },
      remoteRows: { total: 5000, offset: 0, min: -10, max: 100, owner: {}, onRange: (first, count) => fixture.ranges.push([first, count]) } };
    type Node = ReactElement<Record<string, any>>;
    const render = () => {
      cursor = 0; const tree = Chart(props) as Node;
      const scroll = tree.props.children[0] as Node, canvas = scroll.props.children[0] as Node;
      scroll.props.ref.current = box; canvas.props.ref.current = element;
      for (const effect of pending.splice(0)) { const cleanup = effect(); if (typeof cleanup === 'function') cleanups.push(cleanup as () => unknown); }
      return { tree, scroll, canvas };
    };
    const draw = () => task!.frame({ now: 0, dt: 16 });
    const key = (canvas: Node, name: string) => canvas.props.onKeyDown({ key: name, preventDefault() {} });
    let view = render(); draw(); draw();
    assert.equal(fixture.ranges.length, 1, 'unchanged viewport issues one range demand');
    assert.ok(fixture.ranges[0][1] < 25);
    assert.deepEqual(fixture.scales.at(-1), [-10 * 1.06, 106], 'offpage extremes define the stable global axis');
    assert.equal(view.scroll.props.children[1].props.style.height, `calc(${raceHeight(5000)}px - 100%)`);
    key(view.canvas, 'End'); draw();
    assert.ok(box.scrollTop > 99000, 'End uses the original global scroller');
    assert.equal(fixture.tip, null, 'unloaded target has no invented tooltip');
    const [first, count] = fixture.ranges.at(-1)!;
    assert.ok(first < 5000 && first + count === 5000);
    props.items = items(first, count); props.remoteRows = { ...props.remoteRows!, offset: first };
    view = render(); draw();
    assert.equal((fixture.tip as { title: string } | null)?.title, 'Actor 4999');
    key(view.canvas, 'Enter'); assert.deepEqual(picks, ['4999']);
    key(view.canvas, 'ArrowUp'); draw();
    assert.equal((fixture.tip as { title: string } | null)?.title, 'Actor 4998');
    key(view.canvas, 'ArrowDown'); draw();
    assert.equal((fixture.tip as { title: string } | null)?.title, 'Actor 4999');
    assert.match(view.tree.props.children[1].props.children, /5000 ranked entries.*5000\. 4999 1/);
    key(view.canvas, 'Home'); draw();
    listeners.get('pointermove')!({ clientX: 0, clientY: 0 });
    props.items = items(0, 20); props.remoteRows = { ...props.remoteRows!, offset: 0 };
    view = render(); draw(); assert.equal(fixture.tip, null, 'new pointer input cancels delayed keyboard focus');
    key(view.canvas, 'Home'); draw();
    for (let rank = 0; rank < 20; rank++) key(view.canvas, 'ArrowDown');
    draw(); assert.equal(fixture.tip, null, 'crossing the loaded slice waits for a genuine global row');
    const [nextFirst, nextCount] = fixture.ranges.at(-1)!;
    assert.ok(nextFirst <= 20 && nextFirst + nextCount > 20);
    props.items = items(nextFirst, nextCount); props.remoteRows = { ...props.remoteRows!, offset: nextFirst };
    view = render(); draw();
    assert.equal((fixture.tip as { title: string } | null)?.title, 'Actor 20');
    key(view.canvas, 'End');
    props.items = items(4980, 20); props.remoteRows = { ...props.remoteRows!, offset: 4980, owner: {} };
    view = render(); draw(); key(view.canvas, 'Enter');
    assert.deepEqual(picks, ['4999'], 'another numeric snapshot cannot inherit pending focus');
    box.scrollTop = 20000; view.scroll.props.onScroll({ currentTarget: box }); draw();
    assert.ok(fixture.ranges.at(-1)![0] >= 1000 && fixture.ranges.at(-1)![0] < 1020, 'ordinary scroll requests the global visible range');
    props.items = []; props.empty = 'Nothing on this day.'; props.remoteRows = { ...props.remoteRows!, total: 0, offset: 0, owner: {} };
    view = render();
    assert.equal(view.tree.props.children[1].props.children, '0 ranked entries. Nothing on this day.', 'an empty page reads why, not loading');
  } finally {
    for (const cleanup of cleanups) cleanup(); await vite.close(); delete host.__remoteRace;
  }
});
