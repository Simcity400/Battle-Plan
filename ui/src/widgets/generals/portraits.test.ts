import test from 'node:test';
import assert from 'node:assert/strict';
import { portraitSpec } from './portraits.ts';
import { HourStore, planRequests } from '../../dashboard/countryFacts.ts';
import type { UnitLeaderPortrait } from '../../generated/dashboard';
import { createServer } from 'vite';
import type { DashboardClient, ProductRequest } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';

test('mounted portrait requests reuse reads and keep leaderboard and career dimensions separate', () => {
  const leaderboard = portraitSpec(86);
  const career = portraitSpec(118);
  assert.strictEqual(portraitSpec(86), leaderboard);
  assert.notEqual(leaderboard.identity, career.identity);
  assert.deepEqual(career.params('1', 100, ['8']), { branchId: '1', hour: 100, references: ['8'], edge: 118 });
  const store = new HourStore<UnitLeaderPortrait>(Infinity);
  const mounted = [{ hour: 100, tag: '8' }, { hour: 100, tag: '9' }];
  assert.deepEqual(planRequests(store, mounted, Infinity, leaderboard.perRequest), [[100, ['8', '9']]]);
  store.record(100, ['8', '9'], leaderboard.extract({ portraits: [{ reference: '8', image: null, missing: 'Unavailable' }], warnings: [] }));
  assert.deepEqual(planRequests(store, mounted, Infinity, leaderboard.perRequest), []);
  assert.deepEqual(planRequests(store, [...mounted, { hour: 100, tag: '10' }], Infinity, leaderboard.perRequest), [[100, ['10']]]);
  const references = Array.from({ length: 100 }, (_, i) => String(i));
  assert.deepEqual(planRequests(store, references.map(tag => ({ hour: 101, tag })), Infinity, leaderboard.perRequest), [[101, [...references].sort()]]);
});

test('portrait facts reject malformed references, artwork and duplicate entries', () => {
  const extract = portraitSpec(86).extract;
  const portrait = { reference: '8', image: 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png', missing: null };
  assert.deepEqual(extract({ portraits: [portrait], warnings: ['Content is unverified'] }).get('8'), { ...portrait, warnings: ['Content is unverified'] });
  for (const reply of [null, { portraits: null }, { portraits: [null] },
    { portraits: [{ ...portrait, reference: '18446744073709551616' }] },
    { portraits: [{ ...portrait, image: 'https://invalid.example/image.png' }] },
    { portraits: [{ ...portrait, missing: false }] }, { portraits: [portrait, portrait] }]) {
    assert.throws(() => extract(reply && { ...reply, warnings: [] }), /malformed/);
  }
  for (const warnings of [undefined, null, 'Unverified', [false]]) {
    assert.throws(() => extract({ portraits: [portrait], warnings }), /malformed/);
  }
});

// Exercise the production hook's request/store scope, not a reconstructed cache key.
test('actual portrait fact stores retain refusals only within the same edge, recording and content scope', async () => {
  type Effect = { deps: readonly unknown[]; run: () => void | (() => void); cleanup?: () => void };
  const slots = new Map<number, unknown>(), effects = new Map<number, Effect>(), pending = new Map<number, Effect>();
  let cursor = 0;
  const missing: UnitLeaderPortrait = { reference: '8', image: null, missing: 'Synthetic portrait refusal' };
  const same = (a: readonly unknown[], b: readonly unknown[]) => a.length === b.length && a.every((value, index) => Object.is(value, b[index]));
  const fixture = {
    content: 0,
    calls: [] as ProductRequest[],
    reply: { portraits: [missing], warnings: ['Synthetic content warning'] },
    useState(initial: unknown) {
      const index = cursor++;
      if (!slots.has(index)) slots.set(index, initial);
      return [slots.get(index), (next: unknown) => slots.set(index, typeof next === 'function' ? next(slots.get(index)) : next)];
    },
    useRef(initial: unknown) {
      const index = cursor++;
      if (!slots.has(index)) slots.set(index, { current: initial });
      return slots.get(index);
    },
    useMemo(make: () => unknown, deps: readonly unknown[]) {
      const index = cursor++, old = slots.get(index) as { deps: readonly unknown[]; value: unknown } | undefined;
      if (!old || !same(old.deps, deps)) slots.set(index, { deps: [...deps], value: make() });
      return (slots.get(index) as { value: unknown }).value;
    },
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
    unmount() { for (const effect of effects.values()) effect.cleanup?.(); },
    load(request: ProductRequest) { fixture.calls.push(request); return Promise.resolve(fixture.reply); },
  };
  const host = globalThis as typeof globalThis & { __portraitFacts?: typeof fixture };
  host.__portraitFacts = fixture;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'portrait-fact-scope', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/countryNamesAt.ts')) return source.replace(
          "import { useEffect, useMemo, useRef, useState } from 'react';",
          'const { useEffect, useMemo, useRef, useState } = globalThis.__portraitFacts;');
        if (path.endsWith('/dashboard/contentRevision.ts')) return 'export const useContentRevision = () => globalThis.__portraitFacts.content;';
        if (path.endsWith('/dashboard/products.ts')) return 'export const loadTransientProduct = (_client, request) => globalThis.__portraitFacts.load(request);';
      },
    }] });
  const unused = async () => { throw new Error('Unexpected fixture transport call'); };
  const client: DashboardClient = { available: true, openSource: unused, product: unused,
    releaseSource: async () => {}, setMapViews: async () => {}, onMapStates: async () => () => {} };
  const settle = () => new Promise<void>(resolve => setImmediate(resolve));
  try {
    const { useCountryFactsAt } = await vite.ssrLoadModule('/ui/src/dashboard/countryNamesAt.ts') as typeof import('../../dashboard/countryNamesAt.ts');
    const { portraitSpec: specAt } = await vite.ssrLoadModule('/ui/src/widgets/generals/portraits.ts') as typeof import('./portraits.ts');
    let edge = 86;
    const recording = (id: string): Pick<BoundTrack, 'source' | 'branch'> => {
      const branch: BoundTrack['branch'] = { id: '1', sessionId: '1', firstHour: 100, lastHour: 100,
        main: true, fork: null, joins: [], sampleBoundary: 'Synthetic boundary', gaps: [] };
      const path = `memory/${id}.battleplan`;
      return { source: { id, path, revision: '1', metadata: { path, name: id, revision: '1', format: 1, captureMode: 'full',
        campaignKey: 'c0ffee', branches: [branch], countries: [], playerCountryId: null,
        contentVerified: false, partial: null, warnings: [], buildWarnings: [] } }, branch };
    };
    let track = recording('recording-a');
    const wanted = [{ hour: 100, tag: '8' }];
    const render = () => { cursor = 0; return useCountryFactsAt(client, track, wanted, true, specAt(edge)); };
    const portrait = () => {
      const read = render().get('[100,"8"]');
      assert.ok(read && !(read instanceof Error));
      return read;
    };
    assert.equal(render().size, 0); fixture.commit(); await settle();
    assert.deepEqual(render().get('[100,"8"]'), { ...fixture.reply.portraits[0], warnings: fixture.reply.warnings });
    fixture.commit(); await settle();
    assert.equal(fixture.calls.length, 1, 'an answered refusal is cached at the mounted read');
    edge = 118;
    assert.equal(render().size, 0, 'the career edge cannot inherit the leaderboard refusal');
    fixture.commit(); await settle();
    assert.equal(portrait().missing, 'Synthetic portrait refusal');
    assert.deepEqual(fixture.calls.map(call => call.params), [
      { branchId: '1', hour: 100, references: ['8'], edge: 86 },
      { branchId: '1', hour: 100, references: ['8'], edge: 118 },
    ]);
    track = recording('recording-b');
    fixture.reply = { portraits: [{ reference: '8', image: 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png', missing: null }], warnings: [] };
    assert.equal(render().size, 0, 'a reused reference in another recording starts pending');
    fixture.commit(); await settle();
    assert.equal(portrait().missing, null);
    assert.equal(fixture.calls.at(-1)?.sourceId, 'recording-b');
    track = { ...track, source: { ...track.source, revision: '2', metadata: { ...track.source.metadata, revision: '2' } } };
    assert.equal(render().size, 0); fixture.commit(); await settle();
    assert.equal(fixture.calls.at(-1)?.revision, '2');
    fixture.content++;
    assert.equal(render().size, 0, 'changed content cannot retain a prior portrait or refusal');
    fixture.commit(); await settle();
    assert.equal(portrait().image, 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png');
    assert.equal(fixture.calls.length, 5);
  } finally { fixture.unmount(); await vite.close(); delete host.__portraitFacts; }
});
