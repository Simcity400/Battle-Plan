import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { MovingRow } from './rowMotion';

test('a new visible-row consumer receives the mounted rows even when their content is unchanged', async () => {
  const refs: { current: unknown }[] = [];
  const effects: (() => unknown)[] = [];
  let cursor = 0;
  const rows: MovingRow<string>[] = Array.from({ length: 5000 }, (_, slot) => ({ key: `native-${slot}`,
    item: `Country ${slot}`, slot, phase: 'staying', since: 0 }));
  const fixture = {
    rows,
    useRef(value: unknown) { const index = cursor++; return refs[index] ??= { current: value }; },
    useState(value: unknown) { return [value, () => {}]; },
    useLayoutEffect(effect: () => unknown) { effects.push(effect); },
  };
  const host = globalThis as typeof globalThis & { __windowedRows?: typeof fixture };
  host.__windowedRows = fixture;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'windowed-rows-consumer', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/charts/WindowedRows.tsx')) return source.replace(/import \{ ([^}]+) \} from 'react';/,
          'const { $1 } = globalThis.__windowedRows;');
        if (path.endsWith('/charts/useRowMotion.ts')) return 'export const useRowMotion = () => globalThis.__windowedRows.rows; export const movingRowClass = () => "";';
      },
    }] });
  try {
    const { default: WindowedRows } = await vite.ssrLoadModule('/ui/src/charts/WindowedRows.tsx');
    const items = rows.map(({ key, item }) => ({ key, item }));
    const draw = (onVisibleRows?: (rows: readonly MovingRow<string>[]) => void) => {
      cursor = 0; effects.length = 0;
      WindowedRows({ items, rowHeight: 20, label: 'Countries', onVisibleRows, children: () => null });
      for (const effect of effects) effect();
    };
    draw();
    const first: (readonly MovingRow<string>[])[] = [], next: (readonly MovingRow<string>[])[] = [];
    const consumeFirst = (rows: readonly MovingRow<string>[]) => first.push(rows);
    const consumeNext = (rows: readonly MovingRow<string>[]) => next.push(rows);
    draw(consumeFirst);
    assert.equal(first.length, 1, 'attaching a consumer reports the existing viewport');
    assert.ok(first[0].length > 0 && first[0].length < 100);
    draw(consumeFirst);
    assert.equal(first.length, 1, 'the same consumer and content do not create a render loop');
    draw(consumeNext);
    assert.equal(next.length, 1, 'a callback handoff republishes the unchanged mounted rows');
    assert.deepEqual(next[0], first[0]);
    draw(); draw(consumeNext);
    assert.equal(next.length, 2, 'reenabling artwork demand restores its visible rows');
    fixture.rows = rows.map(row => row.slot === 0 ? { ...row, item: 'Renamed country' } : row);
    draw(consumeNext);
    assert.equal(next.at(-1)?.[0].item, 'Renamed country');
  } finally { delete host.__windowedRows; await vite.close(); }
});
