import assert from 'node:assert/strict';
import test from 'node:test';
import * as React from 'react';
import { createServer } from 'vite';

const flush = () => new Promise<void>(resolve => setImmediate(resolve));
function mount<T>(read: () => T) {
  type Slot = { value?: any; initialized?: boolean; deps?: readonly unknown[]; cleanup?: () => void };
  const slots: Slot[] = [];
  let cursor = 0, effects: (() => void)[] = [];
  const internals = (React as any).__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE;
  const changed = (slot: Slot, deps: readonly unknown[]) => !slot.deps || slot.deps.length !== deps.length
    || deps.some((value, index) => !Object.is(value, slot.deps![index]));
  const dispatcher = {
    useState(initial: any) {
      const slot = slots[cursor++] ??= {};
      if (!slot.initialized) { slot.value = typeof initial === 'function' ? initial() : initial; slot.initialized = true; }
      return [slot.value, (next: any) => { slot.value = typeof next === 'function' ? next(slot.value) : next; }];
    },
    useRef(value: unknown) { return (slots[cursor++] ??= { value: { current: value } }).value; },
    useMemo(make: () => unknown, deps: readonly unknown[]) {
      const slot = slots[cursor++] ??= {};
      if (changed(slot, deps)) { slot.value = make(); slot.deps = [...deps]; }
      return slot.value;
    },
    useCallback(callback: unknown, deps: readonly unknown[]) { return dispatcher.useMemo(() => callback, deps); },
    useEffect(effect: () => (() => void) | undefined, deps: readonly unknown[]) {
      const slot = slots[cursor++] ??= {};
      if (changed(slot, deps)) { slot.deps = [...deps]; effects.push(() => { slot.cleanup?.(); slot.cleanup = effect(); }); }
    },
    useSyncExternalStore(_subscribe: unknown, snapshot: () => unknown) { cursor++; return snapshot(); },
  };
  return {
    render() {
      cursor = 0; effects = [];
      const previous = internals.H; internals.H = dispatcher;
      let result: T;
      try { result = read(); } finally { internals.H = previous; }
      for (const effect of effects) effect();
      return result!;
    },
    unmount() { for (const slot of slots) slot?.cleanup?.(); },
  };
}

const tags = Array.from({ length: 5000 }, (_, slot) => `mod / ${slot}`);
const track = { source: { id: 'forces-progressive', revision: '1' }, branch: { id: '1', firstHour: 0, lastHour: 31 } };
function summary(rough: boolean, value: number) {
  return { from: 0, to: 31, width: 32, buckets: [{ firstHour: 0, lastHour: 31, firstValue: value,
    lastValue: value, lastValueHour: 31, minValue: value, minHour: 0, maxValue: value, maxHour: 0,
    firstBreak: true, hasBreak: false }], runs: [{ firstHour: 0, lastHour: 31, value, startsSegment: true }],
    approximate: rough ? [{ firstHour: 0, lastHour: 31 }] : [] };
}
function clientFor(reply: (params: any, complete: boolean) => unknown, limit: number) {
  let complete = false, queries = 0, statuses = 0;
  const calls: string[] = [];
  const waiting = new Set<() => void>();
  const client = {
    preparedQuery: async (_source: unknown, _product: string, params: any) => {
      calls.push(`${_product}:${params.from ?? params.reads?.[0]?.hour}:${params.to ?? params.reads?.length}`);
      assert.ok(++queries <= limit, 'country count must not grow the request count');
      return { ...(reply(params, complete) as object), coverage: complete
        ? { firstHour: 0, lastHour: 31, complete: true } : { firstHour: 0, lastHour: 1, complete: false } };
    },
    preparationStatus: async (input: object, signal: AbortSignal) => {
      assert.ok(++statuses <= limit, 'no status spin');
      await new Promise<void>(resolve => { waiting.add(resolve); signal.addEventListener('abort', () => resolve(), { once: true }); });
      return { ...input, state: 'ready', sections: [], backgroundEnabled: true, bytes: '0', error: null,
        coverage: { '1': { firstHour: 0, lastHour: 31, complete: true } } };
    },
    product: async () => { throw Error('unexpected oracle route'); },
  };
  return { client, calls, counts: () => ({ queries, statuses }), finish() { complete = true; for (const resolve of waiting) resolve(); } };
}

test('army publishes rough geometry for 5000 countries on a short screen before preparation completes', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  const native = clientFor((_params, complete) => ({ series: tags.map((tag, slot) => ({ tag,
    summary: summary(!complete, 7), identities: [{ slot, countryId: slot, firstHour: 0, lastHour: 31 }] })) }), 2);
  let hook: ReturnType<typeof mount<any>> | undefined;
  try {
    const { usePreparedArmy } = await vite.ssrLoadModule('/ui/src/widgets/army-size/preparedArmy.ts');
    hook = mount(() => usePreparedArmy(native.client, track,
      { branchId: '1', tags, measure: 'divisions', countFor: 'owner', byType: false }, undefined, true));
    hook.render().onSummaryView({ from: 0, to: 31, width: 32 });
    hook.render(); await flush();
    const rough = hook.render();
    assert.equal(rough.dense, false);
    assert.equal(rough.summaries.size, 5000, 'short views also request progressive summaries');
    assert.equal(rough.identities.length, 5000);
    assert.deepEqual(rough.summaries.get(tags[0]).approximate, [{ firstHour: 0, lastHour: 31 }]);
    assert.deepEqual(native.counts(), { queries: 1, statuses: 1 });
    native.finish(); await flush();
    assert.deepEqual(hook.render().summaries.get(tags[0]).approximate, []);
    assert.equal(native.counts().queries, 2);
  } finally { hook?.unmount(); native.finish(); await vite.close(); }
});

test('supply keeps rough counts in display geometry and exact counts in reached data at 5000 countries', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  const native = clientFor((params, complete) => {
    const rough = !complete && params.from !== params.to;
    return { branchId: '1', available: true, reason: null, missingInputs: [], warnings: [],
      series: params.tags.map((tag: string, slot: number) => ({ tag, identities: [{ slot, countryId: slot, firstHour: params.from, lastHour: params.to }],
        runs: [{ firstHour: params.from, lastHour: params.to, startsSegment: true, noSupply: rough ? 99 : 1, strained: 0, inSupply: 0 }] })),
      summaries: params.tags.map((tag: string) => ({ tag, values: [summary(rough, 100), summary(rough, 0), summary(rough, 0)] })) };
  }, 8);
  let hook: ReturnType<typeof mount<any>> | undefined;
  try {
    const { usePreparedSupply } = await vite.ssrLoadModule('/ui/src/widgets/supply-health/preparedSupply.ts');
    hook = mount(() => usePreparedSupply(native.client, track, tags, 9, null, true));
    hook.render(); await flush();
    hook.render().onSummaryView({ from: 0, to: 31, width: 32 });
    hook.render(); await flush();
    const rough = hook.render();
    assert.equal(rough.geometry.size, 5000);
    assert.equal(rough.geometry.get(tags[4999])[0].noSupply, 99);
    assert.equal(rough.data.series.length, 5000);
    assert.deepEqual(rough.data.series[4999].runs.map((run: any) => [run.firstHour, run.lastHour, run.noSupply]), [[9, 9, 1]],
      'rough interpolated counts never become hover or drill values');
    assert.ok(native.counts().queries <= 4, native.calls.join(", "));
    native.finish(); await flush();
    assert.deepEqual(hook.render().summaries.get(tags[0])[0].approximate, []);
    assert.ok(native.counts().queries <= 8);
  } finally { hook?.unmount(); native.finish(); await vite.close(); }
});

test('air and navy share rough display publication, exact reached values and approximate stack provenance', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'force-production-helpers', enforce: 'pre',
      transform(source, id) { if (id.replaceAll('\\', '/').endsWith('/charts/race/PartsRaceView.tsx'))
        return source.replace('function mergePreparedParts(', 'export function mergePreparedParts(')
          .replace('function preparedPartsGeometry(', 'export function preparedPartsGeometry('); },
    }] });
  try {
    const { usePreparedRace, mergePreparedParts, preparedPartsGeometry } = await vite.ssrLoadModule('/ui/src/charts/race/PartsRaceView.tsx');
    for (const product of ['air-force-size', 'navy-size']) {
      const native = clientFor((params, complete) => {
        const first = params.reads?.[0]?.hour ?? params.from, last = params.reads?.[0]?.hour ?? params.to;
        const stateRough = !complete && params.reads && new Set(params.reads.map((read: any) => read.hour)).size > 13;
        const rough = !complete && first !== last;
        return { approximate: stateRough ? [{ firstHour: 0, lastHour: 31 }] : [], branchId: '1', parts: [{ key: 'mod class', name: 'Mod class', unresolved: false }], warnings: [],
          series: tags.map((tag, slot) => ({ tag, identities: [{ slot, countryId: slot, firstHour: first, lastHour: last }],
            segments: [{ firstHour: first, lastHour: last }], membership: null,
            runs: [{ firstHour: first, lastHour: last, startsSegment: true, total: stateRough ? 999 : rough ? 99 : 7, values: [stateRough ? 999 : rough ? 99 : 7], referenceTotal: null }] })),
          summaries: tags.map((tag, slot) => {
            const line = summary(rough, 99);
            if (rough) line.buckets[0] = { ...line.buckets[0], lastHour: slot % 16 * 2 + 1, lastValueHour: slot % 16 * 2 + 1 };
            return { tag, summary: line, parts: [{ key: 'mod class', summary: summary(rough, 99) }] };
          }) };
      }, 10);
      const request = { sourceId: track.source.id, revision: '1', product, params: { branchId: '1', tags, measure: 'planes', ownership: 'roster' } };
      const hook = mount(() => usePreparedRace(native.client, track, request, 9, true, (value: unknown) => value,
        mergePreparedParts, preparedPartsGeometry, true));
      try {
        hook.render(); await flush();
        hook.render().onSummaryView({ from: 0, to: 31, width: 32 });
        hook.render(); await flush();
        let rough = hook.render(); await flush(); rough = hook.render();
        assert.equal(rough.dense, false);
        assert.equal(rough.geometry.series.length, 5000);
        assert.equal(rough.geometry.series[4999].runs[0].total, 99);
        assert.equal(rough.data.series.length, 5000);
        assert.ok(rough.data.series[4999].runs.every((run: any) => run.total === 7));
        assert.deepEqual(rough.approximate.get(tags[4999]), [{ firstHour: 0, lastHour: 31 }]);
        assert.equal(native.counts().queries, 5, native.calls.join(", "));
        native.finish(); await flush();
        assert.deepEqual(hook.render().approximate.get(tags[0]), []);
        assert.ok(native.counts().queries <= 10);
      } finally { hook.unmount(); native.finish(); }
    }
  } finally { await vite.close(); }
});
