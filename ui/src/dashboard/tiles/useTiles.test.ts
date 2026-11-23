import assert from 'node:assert/strict';
import test from 'node:test';
import * as React from 'react';
import type { SeriesTileReply } from '../../generated/dashboard';
import { TILE_BINS } from './tile.ts';
import { useTiles } from './useTiles.ts';
import { gatedWindow } from '../../test-support/gated.ts';

function mount<T>(read: () => T) {
  type Slot = { value?: any; initialized?: boolean; deps?: readonly unknown[]; cleanup?: () => void };
  const slots: Slot[] = [];
  let cursor = 0, effects: (() => void)[] = [], changes = 0;
  const internals = (React as any).__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE;
  const changed = (slot: Slot, deps: readonly unknown[]) => !slot.deps || slot.deps.length !== deps.length
    || deps.some((value, index) => !Object.is(value, slot.deps![index]));
  const dispatcher = {
    useState(initial: any) {
      const slot = slots[cursor++] ??= {};
      if (!slot.initialized) { slot.value = typeof initial === 'function' ? initial() : initial; slot.initialized = true; }
      return [slot.value, (next: any) => {
        const value = typeof next === 'function' ? next(slot.value) : next;
        if (!Object.is(value, slot.value)) changes++;
        slot.value = value;
      }];
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
    changes: () => changes,
    unmount() { for (const slot of slots) slot?.cleanup?.(); },
  };
}

const settle = () => new Promise<void>(resolve => setImmediate(resolve));

test('a chart on tiles renders when a tile lands, never because its view or the head moved', async () => {
  const asked: number[] = [];
  const client = {
    product: async (request: { sourceId: string; revision: string; product: string; params: { binHours: number; index: number; countries: string[] } }) => {
      const { binHours, index, countries: tags } = request.params;
      asked.push(index);
      const first = index * TILE_BINS * binHours;
      const reply: SeriesTileReply = { branchId: '1', measure: 'politicalPower', decimals: 0, binHours, index, firstHour: first, immutable: true,
        filled: first + TILE_BINS * binHours - 1, rawValues: true, hourBytes: 1,
        lines: tags.map(tag => ({ country: tag, missing: [{ firstHour: first, lastHour: first + TILE_BINS * binHours - 1, reason: 'not recorded' }], parts: [] })),
        bins: btoa('\0'.repeat(TILE_BINS * tags.length)) };
      return { sourceId: request.sourceId, revision: request.revision, product: request.product, data: reply };
    },
  } as unknown as Parameters<typeof useTiles>[0];
  const series = { source: { id: 'source', revision: '1' }, branchId: '1', family: { kind: 'country-measure', measure: 'politicalPower' },
    tags: ['GER', 'ITA'], span: (first: number, last: number) => gatedWindow(first, last), lastHour: 100_000 };
  const hook = mount(() => useTiles(client, series, true));
  try {
    hook.render().onView({ from: 0, to: 8759, end: 8759, width: 600 });
    await settle();
    assert.ok(hook.render().lines, 'the lines are assembled once their tiles land');
    const landed = hook.changes(), tiles = asked.length;
    for (let head = 8760; head + 32 * 12 < 3 * TILE_BINS * 12; head += 1) {
      hook.render().onView({ from: head - 8759, to: head, end: head, width: 600 });
      await settle();
    }
    assert.equal(asked.length, tiles, 'nothing asked inside the tiles held');
    assert.equal(hook.changes(), landed, 'no render while the head moves inside them');
    assert.ok(hook.render().lines!.get('GER')!.summary.buckets.every(bucket => bucket === null));
    assert.equal(hook.render().lines!.get('GER')!.missing[0].reason, 'not recorded');
  } finally { hook.unmount(); }
});
