import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { TimelineProps } from './Timeline';

test('prepared keyboard stops queue exact reads, use their hour for Jump, and discard departed selections', async () => {
  const refs: { current: unknown }[] = [];
  const hooks = { useId: () => 'prepared-navigation', useMemo: (build: () => unknown) => build(),
    useRef(value: unknown) { const ref = { current: value }; refs.push(ref); return ref; },
    useState: (value: unknown) => [value, () => {}], useEffect() {} };
  const host = globalThis as typeof globalThis & { __preparedNavigation?: typeof hooks };
  host.__preparedNavigation = hooks;
  const globals = ['window', 'innerHeight'].map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const);
  Object.defineProperty(globalThis, 'window', { configurable: true, value: { innerHeight: 500, scrollBy() {}, addEventListener() {}, removeEventListener() {} } });
  Object.defineProperty(globalThis, 'innerHeight', { configurable: true, value: 500 });
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'prepared-navigation', enforce: 'pre',
      transform(source, id) {
        if (id.replaceAll('\\', '/').endsWith('/timeline/Timeline.tsx')) return source.replace(/import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__preparedNavigation;');
      },
    }] });
  let hide: (() => void) | undefined;
  try {
    const { default: Timeline } = await vite.ssrLoadModule('/ui/src/charts/timeline/Timeline.tsx');
    const { hideTooltip } = await vite.ssrLoadModule('/ui/src/charts/tooltip.ts'); hide = () => hideTooltip('prepared-navigation');
    const requests: { from: number; forward: boolean; resolve: (hour: number | null) => void }[] = [];
    const shown: number[] = [], jumped: number[] = [];
    const props: TimelineProps = { label: 'Autonomy', revision: 0, extent: [0, 20], playhead: null, showIcons: false, showNames: false, active: true,
      lanes: [{ id: 'holder', label: 'Holder', spans: [{ id: 'level', firstHour: 0, lastHour: 20, label: 'Level' }] },
        { id: 'changes', label: 'Changes', spans: [{ id: 'a', firstHour: 0, lastHour: 9, label: 'A' }, { id: 'b', firstHour: 10, lastHour: 19, label: 'B' }, { id: 'c', firstHour: 20, lastHour: 20, label: 'C' }] }],
      tooltip: (_lane, _span, hour) => { shown.push(hour); return { title: `Score at ${hour}`, rows: [] }; },
      onNavigateHour: (_lane, from, forward) => new Promise(resolve => requests.push({ from, forward, resolve })),
      onContextMenu: hour => jumped.push(hour) };
    type Node = { props: { children?: unknown; onKeyDown?: (event: unknown) => void; onBlur?: () => void } };
    const descendants = (value: unknown): Node[] => Array.isArray(value) ? value.flatMap(descendants)
      : value && typeof value === 'object' && 'props' in value ? [value as Node, ...descendants((value as Node).props.children)] : [];
    const tree = Timeline(props);
    const element = { parentElement: null, offsetHeight: 100, getBoundingClientRect: () => ({ left: 0, top: 0, width: 600, height: 100 }) };
    for (const ref of refs.slice(0, 3)) ref.current = element;
    const canvas = descendants(tree).find(node => node.props.onKeyDown)!;
    const key = (key: string) => canvas.props.onKeyDown!({ key, preventDefault() {} });
    const tick = async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); };
    key('ArrowLeft'); key('ArrowLeft'); await tick();
    assert.deepEqual(requests.map(({ from, forward }) => [from, forward]), [[21, false]]);
    requests[0].resolve(17); await tick();
    assert.deepEqual(requests.map(({ from, forward }) => [from, forward]), [[21, false], [17, false]]);
    requests[1].resolve(9); await tick();
    assert.ok(shown.includes(17) && shown.includes(9));
    key('ContextMenu'); assert.deepEqual(jumped, [9]);
    key('ArrowRight'); await tick(); canvas.props.onBlur!(); requests[2].resolve(17); await tick();
    assert.equal(shown.at(-1), 9, 'blur discards the pending stop');
    key('ArrowLeft'); await tick(); requests[3].resolve(null); await tick();
    assert.ok(shown.every(hour => hour <= 20), 'a null first stop never exposes head + 1');
    key('ArrowLeft'); await tick(); key('Escape'); requests[4].resolve(7); await tick();
    assert.notEqual(shown.at(-1), 7);
    // A lane changed to shows its last change, at 20; Left asks for the change before that one.
    key('ArrowDown'); key('ArrowDown'); key('ArrowLeft'); await tick();
    assert.deepEqual([requests.at(-1)!.from, requests.at(-1)!.forward], [20, false]);
  } finally {
    hide?.(); delete host.__preparedNavigation; await vite.close();
    for (const [key, descriptor] of globals) { if (descriptor) Object.defineProperty(globalThis, key, descriptor); else Reflect.deleteProperty(globalThis, key); }
  }
});
