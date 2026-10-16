import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { tokensFromValues } from '../tokens.ts';
import type { FrameTask } from '../scheduler';
import type { RaceChartProps } from './RaceChart';
import { MIN_LABEL, NAME_PAD, VALUE_PAD } from './draw.ts';

test('race name columns retain the same unavailable-name room with cold or ready labels', async () => {
  const widths: number[] = [];
  const effects: (() => unknown)[] = [];
  const context = new Proxy({ measureText: (text: string) => ({ width: text.length * 7 }) },
    { get: (target, key) => Reflect.get(target, key) ?? (() => {}) });
  const canvas = { width: 0, height: 0, getContext: () => context, addEventListener() {}, removeEventListener() {} };
  const fixture = { canvas, effects, widths, latest: undefined as { current: RaceChartProps } | undefined, task: undefined as FrameTask | undefined };
  const host = globalThis as typeof globalThis & { __raceGeometry?: typeof fixture; __raceGeometryEffect?: (effect: () => unknown) => void };
  host.__raceGeometry = fixture; host.__raceGeometryEffect = effect => { effects.push(effect); };
  const tokens = tokensFromValues(() => '#123456', 'sans-serif', undefined,
    { motion: 'smooth', reducedMotion: true, countryColours: 'raw' });
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'race-name-geometry', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/charts/tokens.ts')) return `export const readTokens = () => (${JSON.stringify(tokens)}); export const subscribeTokens = () => () => {};`;
        if (path.endsWith('/charts/surface.ts')) return `export const toLocal = () => ({ x: 0, y: 0 }); export const watchCanvasSize = () => ({ read: () => ({ layoutWidth: 1000, layoutHeight: 300, backing: { width: 1000, height: 300, scaleX: 1, scaleY: 1 } }), disconnect() {} });`;
        if (path.endsWith('/charts/scheduler.ts')) return 'export const scheduler = { add(task) { globalThis.__raceGeometry.task = task; return () => {}; }, wake() {}, setVisible() {} };';
        if (path.endsWith('/charts/race/draw.ts')) return `export const FLAG = { x: 8, width: 21, height: 14 }; export const NAME_X = 36; export const VALUE_GAP = 8; export const MIN_LABEL = ${MIN_LABEL}; export const NAME_PAD = ${NAME_PAD}; export const VALUE_PAD = ${VALUE_PAD};
          export const drawDateOverlay = () => {}; export const drawRaceBar = (_ctx, layout) => layout.left;
          export const drawRaceGrid = (_ctx, layout) => globalThis.__raceGeometry.widths.push(layout.left);`;
        if (path.endsWith('/charts/race/RaceChart.tsx')) return source
          .replace('canvas = useRef<HTMLCanvasElement>(null)', 'canvas = useRef(globalThis.__raceGeometry.canvas)')
          .replace('latest.current = props;', 'latest.current = props; globalThis.__raceGeometry.latest = latest;')
          .replace(/\buseEffect\(/g, 'globalThis.__raceGeometryEffect(');
      },
    }] });
  try {
    const { default: Chart } = await vite.ssrLoadModule('/ui/src/charts/race/RaceChart.tsx') as typeof import('./RaceChart');
    const fallback = 'Country name unavailable';
    const origins: number[] = [];
    for (const ready of [false, true]) {
      let name = ready ? 'Recorded name' : fallback;
      const props: RaceChartProps & { nameSamples: readonly string[] } = { label: 'Synthetic country race', active: true, slots: 1,
        items: [{ key: 'country', value: 10, label: '10', stretch: 0, segments: [{ id: 'total', value: 10, color: '#123456' }] }],
        name: () => name, nameSamples: [fallback] };
      effects.length = 0;
      renderToStaticMarkup(createElement(Chart, props));
      const cleanups = effects.map(effect => effect()).filter((cleanup): cleanup is () => void => typeof cleanup === 'function');
      try {
        fixture.task!.frame({ now: 0, dt: 0 });
        name = 'Recorded name';
        fixture.latest!.current = { ...props, name: () => name };
        fixture.task!.frame({ now: 16, dt: 16 });
        origins.push(widths.at(-1)!);
        name = 'A longer recorded country name';
        fixture.latest!.current = { ...props, name: () => name };
        fixture.task!.frame({ now: 32, dt: 16 });
        assert.equal(widths.at(-1), 36 + name.length * 7 + 12, 'authored names grow beyond the unavailable-name floor');
        const nextSample = 'An even longer unavailable country name sample';
        fixture.latest!.current = { ...fixture.latest!.current, nameSamples: [nextSample] };
        fixture.task!.frame({ now: 48, dt: 16 });
        assert.equal(widths.at(-1), 36 + nextSample.length * 7 + 12, 'changed name samples invalidate cached column measurements');
      } finally { for (const cleanup of cleanups.reverse()) cleanup(); }
    }
    assert.equal(origins[0], 36 + fallback.length * 7 + 12);
    assert.equal(origins[1], origins[0], 'ready facts use the same name-column floor as the first cold frame');
  } finally {
    await vite.close(); delete host.__raceGeometry; delete host.__raceGeometryEffect;
  }
});
