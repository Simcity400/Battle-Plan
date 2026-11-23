import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { DashboardClient, DashboardSource } from './client';
import type { PlaybackController } from './playback';
import type { BoundHour } from './regionTime';

test('re-rendering a log whose spec arrived after mount keeps partial pages referentially stable', async () => {
  const states = new Map<number, unknown>();
  const refs = new Map<number, { current: unknown }>();
  const memos = new Map<number, { deps: readonly unknown[]; value: unknown }>();
  const effects = new Map<number, { deps?: readonly unknown[]; cleanup?: () => void }>();
  let pending: (() => void)[] = [];
  let cursor = 0;
  let updates = 0;
  const sameDeps = (a: readonly unknown[] | undefined, b: readonly unknown[] | undefined) =>
    a !== undefined && b !== undefined && a.length === b.length && a.every((value, index) => Object.is(value, b[index]));
  const hooks = {
    useState(initial: unknown) {
      const index = cursor++;
      if (!states.has(index)) states.set(index, typeof initial === 'function' ? (initial as () => unknown)() : initial);
      return [states.get(index), (next: unknown) => {
        const before = states.get(index);
        const value = typeof next === 'function' ? (next as (prev: unknown) => unknown)(before) : next;
        if (!Object.is(before, value)) {
          states.set(index, value);
          updates++;
        }
      }];
    },
    useRef(initial: unknown) {
      const index = cursor++;
      if (!refs.has(index)) refs.set(index, { current: initial });
      return refs.get(index)!;
    },
    useMemo(make: () => unknown, deps: readonly unknown[]) {
      const index = cursor++;
      const before = memos.get(index);
      if (!before || !sameDeps(before.deps, deps)) memos.set(index, { deps: [...deps], value: make() });
      return memos.get(index)!.value;
    },
    useCallback(fn: unknown, deps: readonly unknown[]) {
      return hooks.useMemo(() => fn, deps);
    },
    useEffect(run: () => void | (() => void), deps?: readonly unknown[]) {
      const index = cursor++;
      const before = effects.get(index);
      if (!before || !sameDeps(before.deps, deps)) {
        pending.push(() => {
          effects.get(index)?.cleanup?.();
          const cleanup = run();
          effects.set(index, { deps: deps ? [...deps] : undefined, cleanup: typeof cleanup === 'function' ? cleanup : undefined });
        });
      }
    },
  };
  const counts = { firstHour: 100, binHours: 6, counts: [4], total: 4, before: 0, missing: [] };
  const page = {
    columns: [],
    firstOrdinal: '0',
    total: '4',
    rows: [
      { key: '0', hour: 100, ordinal: '0', cells: [] },
      { key: '1', hour: 101, ordinal: '1', cells: [] },
      { key: '2', hour: 104, ordinal: '2', cells: [] },
      { key: '3', hour: 105, ordinal: '3', cells: [] },
    ],
  };
  const globals = globalThis as typeof globalThis & { __logPagesTest?: { hooks: typeof hooks; counts: typeof counts; page: typeof page } };
  globals.__logPagesTest = { hooks, counts, page };
  const vite = await createServer({
    configFile: false,
    optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{
      name: 'use-log-pages-test',
      enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/useLogPages.ts')) {
          return source
            .replace("import { useCallback, useEffect, useMemo, useRef, useState } from 'react';",
              'const { useCallback, useEffect, useMemo, useRef, useState } = globalThis.__logPagesTest.hooks;')
            .replace("import { loadProduct, useProduct } from './products.ts';",
              'const useProduct = (_client, request) => ({ data: request ? globalThis.__logPagesTest.counts : undefined, error: undefined, retry: () => {} });'
              + ' const loadProduct = () => Promise.resolve(globalThis.__logPagesTest.page);')
            .replace("import { useAheadHour } from './aheadHour.ts';",
              'const useAheadHour = () => ({ hour: 101, until: 102 });')
            .replace("import { inFrame } from './frameUpdates.ts';",
              'const inFrame = (run) => run();');
        }
      },
    }],
  });
  try {
    const { useLogPages } = await vite.ssrLoadModule('/ui/src/dashboard/useLogPages.ts') as typeof import('./useLogPages.ts');
    const source = { id: 'src', revision: '1' } as unknown as DashboardSource;
    const client = {} as unknown as DashboardClient;
    const clock = {
      getHour: () => 101,
      getSnapshot: () => ({ position: 101, hour: 101, fraction: 0, playing: false, hoursPerSecond: 1 }),
    } as unknown as PlaybackController;
    const spec = { source, branchId: '1', log: 'feed-events', firstHour: 100 as BoundHour, lastHour: 105 as BoundHour };
    const cut = { follow: false as const, hour: 101 };
    const row = (reply: { key: string; hour: number }) => ({ key: reply.key, hour: reply.hour, item: reply.key });
    const render = (currentSpec: typeof spec | null) => {
      cursor = 0;
      const result = useLogPages(client, clock, currentSpec, cut, true, row, 4);
      const ran = pending;
      pending = [];
      for (const effect of ran) effect();
      return result;
    };

    render(null);
    render(spec);
    await new Promise<void>(resolve => setImmediate(resolve));
    assert.ok(updates > 0, 'loading the first page triggers a commit');

    const first = render(spec);
    assert.equal(first.pages.length, 1);
    assert.equal(first.pages[0].rows?.length, 2, 'cut at hour 101 shows a partial page of 2 out of 4 rows');

    const second = render(spec);
    assert.equal(second.pages, first.pages, 'an unrelated re-render keeps the exact same partial pages reference');
  } finally {
    for (const effect of effects.values()) effect.cleanup?.();
    delete globals.__logPagesTest;
    await vite.close();
  }
});
