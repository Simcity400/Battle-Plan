import { openGate } from '../test-support/gated.ts';
import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { DashboardClient, ProductReply, ProductRequest } from './client';
import type { CountryStyleParams, CountryStyleReply } from '../generated/dashboard';
import type { BoundTrack } from './model';

type StyleRequest = Omit<ProductRequest, 'params'> & { params: CountryStyleParams };
const ANY_HOUR = openGate(Number.MIN_SAFE_INTEGER, Number.MAX_SAFE_INTEGER);

type Effect = { deps: readonly unknown[]; run: () => void | (() => void); cleanup?: () => void };
const settle = () => new Promise<void>(resolve => setImmediate(resolve));

test('line styles read 439 and 1,500 countries in one request and commit the complete reply', async () => {
  let cursor = 0;
  const slots = new Map<number, unknown>(), effects = new Map<number, Effect>(), pending = new Map<number, Effect>();
  const same = (a: readonly unknown[], b: readonly unknown[]) => a.length === b.length && a.every((value, index) => Object.is(value, b[index]));
  const fixture = {
    useState(initial?: unknown) {
      const index = cursor++;
      if (!slots.has(index)) slots.set(index, initial);
      return [slots.get(index), (next: unknown) => slots.set(index, typeof next === 'function' ? next(slots.get(index)) : next)];
    },
    useRef(initial?: unknown) {
      const index = cursor++;
      if (!slots.has(index)) slots.set(index, { current: initial });
      return slots.get(index);
    },
    useMemo(make: () => unknown, deps: readonly unknown[]) {
      const index = cursor++, old = slots.get(index) as { deps: readonly unknown[]; value: unknown } | undefined;
      if (!old || !same(old.deps, deps)) slots.set(index, { deps: [...deps], value: make() });
      return (slots.get(index) as { value: unknown }).value;
    },
    useContext() { return null; },
    useEffect(run: Effect['run'], deps: readonly unknown[]) {
      const index = cursor++, old = effects.get(index);
      if (!old || !same(old.deps, deps)) pending.set(index, { run, deps: [...deps] });
    },
    commit() {
      for (const [index, effect] of pending) {
        effects.get(index)?.cleanup?.();
        const cleanup = effect.run();
        effect.cleanup = typeof cleanup === 'function' ? cleanup : undefined;
        effects.set(index, effect);
      }
      pending.clear();
    },
    unmount() {
      for (const effect of effects.values()) effect.cleanup?.();
      slots.clear(); effects.clear(); pending.clear();
    },
  };
  const host = globalThis as typeof globalThis & { __lineStylesFixture?: typeof fixture };
  host.__lineStylesFixture = fixture;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'line-styles-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/heldFailures.ts')) return source.replace("import { createContext, useContext, useEffect, useRef, useState } from 'react';",
          "import { createContext } from 'react'; const { useContext, useEffect, useRef, useState } = globalThis.__lineStylesFixture;");
        if (['useCountryLineStyles', 'products'].some(name => path.endsWith(`/dashboard/${name}.ts`)))
          return source.replace(/import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__lineStylesFixture;');
        if (path.endsWith('/dashboard/contentRevision.ts')) return source.replace(
          'return useSyncExternalStore(subscribe, snapshot, snapshot);', 'return snapshot();');
      },
    }] });
  try {
    const { useCountryLineStyles } = await vite.ssrLoadModule('/ui/src/dashboard/useCountryLineStyles.ts') as typeof import('./useCountryLineStyles.ts');
    for (const count of [439, 1_500]) {
      const calls: { input: StyleRequest; answer: () => void }[] = [];
      const client = { product: (input: StyleRequest) => new Promise<ProductReply<CountryStyleReply>>(resolve => {
        assert.equal(input.product, 'country-styles');
        assert.ok(input.params.tags.length > 0);
        calls.push({ input, answer: () => resolve({ ...input, data: { branchId: input.params.branchId, hour: input.params.hour,
          styles: input.params.tags.map(tag => ({ tag, name: `Name ${tag}`, color: [1, 2, 3] })),
          warnings: ['Shared content refusal', `${input.params.tags[0]}: independent refusal`] } }) });
      }) } as unknown as DashboardClient;
      const track = { source: { id: `source-${count}`, revision: 'revision' }, branch: { id: 'branch', firstHour: 0, lastHour: 3000, gaps: [] } } as unknown as BoundTrack;
      const tags = Array.from({ length: count }, (_, index) => `Synthetic country ${String(index).padStart(5, '0')}`);
      const render = (hour = 1, selected = tags) => { cursor = 0; return useCountryLineStyles(client, ANY_HOUR, track, selected, hour, true); };
      render(); fixture.commit(); await settle();
      assert.equal(calls.length, 1, 'one request names every country');
      assert.deepEqual(calls.flatMap(call => call.input.params.tags), tags, 'every opaque country tag is requested exactly once');
      assert.equal(render().data, undefined, 'nothing is presented before the reply');
      calls[0]!.answer(); await settle();
      let current = render(); fixture.commit(); current = render();
      assert.equal(current.data!.styles.length, count);
      assert.equal(current.known.size, count);
      assert.equal(current.data!.warnings.length, calls.length + 1, 'shared warnings deduplicate while every country refusal survives');
      assert.equal(current.known.get(tags.at(-1)!)!.name, `Name ${tags.at(-1)}`);
      const completedReads = calls.length;
      render(1, [...tags].reverse().concat(tags[0])); fixture.commit(); await settle();
      assert.equal(calls.length, completedReads, 'reordering/duplicating a selection reuses identical cached reads');
      current = render(721); fixture.commit(); await settle();
      assert.equal(current.data, undefined);
      assert.equal(current.known.size, count, 'pending monthly refresh holds all previous styles');
      assert.equal(calls.length, completedReads * 2);
      calls.slice(completedReads).forEach(call => call.answer()); await settle();
      current = render(721); fixture.commit(); current = render(721);
      assert.equal(current.data!.hour, 720);
      assert.equal(current.known.size, count);
      fixture.unmount();
    }
    const asked: number[] = [];
    const gapClient = { product: (input: StyleRequest) => {
      asked.push(input.params.hour);
      return Promise.resolve({ ...input, data: { branchId: input.params.branchId, hour: input.params.hour, styles: [], warnings: [] } });
    } } as unknown as DashboardClient;
    const gapped = { source: { id: 'gapped-source', revision: 'revision' },
      branch: { id: 'branch', firstHour: 0, lastHour: 3000, gaps: [{ firstHour: 710, lastHour: 730 }] } } as unknown as BoundTrack;
    cursor = 0; useCountryLineStyles(gapClient, ANY_HOUR, gapped, ['Synthetic country'], 731, true); fixture.commit(); await settle();
    assert.deepEqual(asked, [731]);
    fixture.unmount();
    const slotAsked: string[][] = [];
    const slotClient = { product: (input: StyleRequest) => {
      if (input.product === 'country-identities') return Promise.resolve({ ...input, data: { countries: [
        { tag: 'GER', identities: [{ firstHour: 0, lastHour: 3000, slot: 5, countryId: 50 }], unresolved: [] }] } });
      slotAsked.push(input.params.tags);
      return Promise.resolve({ ...input, data: { branchId: input.params.branchId, hour: input.params.hour,
        styles: [{ tag: 'GER', name: 'Germany', color: [1, 2, 3] }], warnings: [] } });
    } } as unknown as DashboardClient;
    const slotTrack = { source: { id: 'slot-source', revision: 'revision' }, branch: { id: 'branch', firstHour: 0, lastHour: 3000, gaps: [] } } as unknown as BoundTrack;
    const slotKey = '\u00005';
    let shown!: ReturnType<typeof useCountryLineStyles>;
    for (let step = 0; step < 4; step++) { cursor = 0; shown = useCountryLineStyles(slotClient, ANY_HOUR, slotTrack, [slotKey], 1, true); fixture.commit(); await settle(); }
    assert.deepEqual(slotAsked, [['GER']], 'asked by the tag the slot goes by, never by the raw slot key');
    assert.equal(shown.known.get(slotKey)?.name, 'Germany', 'the colour comes back under the key it was asked by');
    fixture.unmount();
    const lateAsked: { hour: number; tags: string[] }[] = [];
    const lateClient = { product: (input: StyleRequest) => {
      if (input.product === 'country-identities') return Promise.resolve({ ...input, data: { countries: [
        { tag: 'GER', identities: [{ firstHour: 0, lastHour: 799, slot: 7, countryId: 70 }, { firstHour: 800, lastHour: 3000, slot: 5, countryId: 50 }], unresolved: [] },
        { tag: 'ITA', identities: [{ firstHour: 0, lastHour: 3000, slot: 9, countryId: 90 }], unresolved: [] }] } });
      lateAsked.push({ hour: input.params.hour, tags: input.params.tags });
      return Promise.resolve({ ...input, data: { branchId: input.params.branchId, hour: input.params.hour,
        styles: input.params.tags.map(tag => ({ tag, name: input.params.hour < 800 ? `Old ${tag}` : `New ${tag}`, color: [1, 2, 3] })), warnings: [] } });
    } } as unknown as DashboardClient;
    const lateTrack = { source: { id: 'late-source', revision: 'revision' }, branch: { id: 'branch', firstHour: 0, lastHour: 3000, gaps: [] } } as unknown as BoundTrack;
    for (let step = 0; step < 4; step++) { cursor = 0; shown = useCountryLineStyles(lateClient, ANY_HOUR, lateTrack, [slotKey, 'ITA'], 900, true); fixture.commit(); await settle(); }
    assert.deepEqual(lateAsked.sort((a, b) => a.hour - b.hour), [{ hour: 720, tags: ['ITA'] }, { hour: 800, tags: ['GER'] }]);
    assert.equal(shown.known.get(slotKey)?.name, 'New GER', 'the slot takes its own colour, not the tag\'s holder at the sample');
    assert.equal(shown.known.get('ITA')?.name, 'Old ITA');
    fixture.unmount();
  } finally { fixture.unmount(); delete host.__lineStylesFixture; await vite.close(); }
});
