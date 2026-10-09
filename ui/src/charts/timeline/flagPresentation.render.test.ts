import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { TimelineProps } from './Timeline';

test('timeline keyboard descriptions keep all 5,000 rows but decode only fitted flags', async () => {
  const refs: { current: unknown }[] = [];
  const fixture = { useId: () => 'timeline-flags', useMemo: (build: () => unknown) => build(),
    useRef(value: unknown) { const ref = { current: value }; refs.push(ref); return ref; },
    useState: (value: unknown) => [value, () => {}], useEffect() {} };
  const host = globalThis as typeof globalThis & { __timelineFlagHooks?: typeof fixture };
  host.__timelineFlagHooks = fixture;
  const globals = ['Image', 'window', 'innerHeight'].map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const);
  let decoded = 0;
  Object.defineProperty(globalThis, 'Image', { configurable: true, value: class { complete = false; naturalWidth = 0; src = '';
    constructor() { decoded++; } } });
  Object.defineProperty(globalThis, 'window', { configurable: true, value: { innerHeight: 500, scrollBy() {}, addEventListener() {}, removeEventListener() {} } });
  Object.defineProperty(globalThis, 'innerHeight', { configurable: true, value: 500 });
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'timeline-fitted-flags', enforce: 'pre',
      transform(source, id) {
        if (id.replaceAll('\\', '/').endsWith('/timeline/Timeline.tsx')) return source.replace(/import \{ ([^}]+) \} from 'react';/,
          'const { $1 } = globalThis.__timelineFlagHooks;');
      },
    }] });
  let hide: (() => void) | undefined;
  try {
    const { default: Timeline } = await vite.ssrLoadModule('/ui/src/charts/timeline/Timeline.tsx');
    const { tooltipStore, hideTooltip, measureTooltipScroll } = await vite.ssrLoadModule('/ui/src/charts/tooltip.ts');
    hide = () => hideTooltip('timeline-flags');
    const rows = Array.from({ length: 5000 }, (_, index) => ({ label: `Participant ${index}`,
      countryFlag: { flag: `synthetic-timeline-member-${index}` } }));
    const props: TimelineProps = { label: 'Participants', revision: 0, extent: [0, 10], playhead: null, showIcons: false, showNames: true, active: true,
      lanes: [{ id: 'lane', label: 'Lane', spans: [{ id: 'span', label: 'Span', firstHour: 0, lastHour: 10 }] }],
      tooltip: () => ({ title: 'Battle participants', rows }) };
    type Node = { props: { children?: unknown; onKeyDown?: (event: unknown) => void } };
    const descendants = (value: unknown): Node[] => Array.isArray(value) ? value.flatMap(descendants)
      : value && typeof value === 'object' && 'props' in value ? [value as Node, ...descendants((value as Node).props.children)] : [];
    const tree = Timeline(props);
    const element = { parentElement: null, offsetHeight: 100, getBoundingClientRect: () => ({ left: 0, top: 0, width: 600, height: 100 }) };
    for (const ref of refs.slice(0, 3)) ref.current = element;
    const key = (value: string) => descendants(tree).find(node => node.props.onKeyDown)!.props.onKeyDown!({ key: value, preventDefault() {} });
    key('ArrowDown');
    const shown = tooltipStore.get()!.content;
    assert.ok(decoded > 0 && decoded < 100, 'hidden participant rows never start Image decoders');
    assert.ok(shown.rows.length < 100);
    assert.match(shown.spoken, /Participant 4999/);
    props.tooltip = () => ({ title: 'Battle participants', rows, scrollable: true, scrollId: 'battle' });
    key('ArrowRight');
    assert.equal(tooltipStore.get()!.content.rows, rows);
    measureTooltipScroll('timeline-flags', 120);
    key('PageDown');
    assert.equal(tooltipStore.get()!.scrollTop, 120);
  } finally {
    hide?.(); delete host.__timelineFlagHooks; await vite.close();
    for (const [key, descriptor] of globals) { if (descriptor) Object.defineProperty(globalThis, key, descriptor); else Reflect.deleteProperty(globalThis, key); }
  }
});
