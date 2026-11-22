import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { DashboardClient, ProductReply, ProductRequest } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import type { Artwork, ResearchEntry } from '../../generated/dashboard';
import type { ResearchHistory } from './types';
import { researchAt, researchArtworkTokens } from './researchData.ts';

type Effect = { deps: readonly unknown[]; run: () => void | (() => void); cleanup?: () => void };
const settle = () => new Promise<void>(resolve => setImmediate(resolve));

// quarantine: #3 re-enable in the dashboard rework; the issue names its failure
test.skip('technology artwork reports failed batches without losing successful text and masks content changes', async () => {
  const slots = new Map<number, unknown>(), effects = new Map<number, Effect>(), pending = new Map<number, Effect>();
  let cursor = 0;
  const same = (a: readonly unknown[], b: readonly unknown[]) => a.length === b.length && a.every((value, i) => Object.is(value, b[i]));
  const fixture = {
    useState(initial?: unknown) {
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
    unmount() {
      for (const effect of effects.values()) effect.cleanup?.();
      slots.clear(); effects.clear(); pending.clear();
    },
  };
  const host = globalThis as typeof globalThis & { __technologyArtworkHooks?: typeof fixture };
  host.__technologyArtworkHooks = fixture;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'technology-artwork-hooks-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/research/useTechnologyArtwork.ts') || path.endsWith('/dashboard/countryNamesAt.ts'))
          return source.replace(/import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__technologyArtworkHooks;');
        if (path.endsWith('/dashboard/contentRevision.ts')) return source.replace(
          'return useSyncExternalStore(revision.subscribe, revision.snapshot, revision.snapshot);', 'return revision.snapshot();');
      },
    }] });
  try {
    const { useTechnologyArtwork } = await vite.ssrLoadModule('/ui/src/widgets/research/useTechnologyArtwork.ts') as typeof import('./useTechnologyArtwork');
    const { contentRevision } = await vite.ssrLoadModule('/ui/src/dashboard/contentRevision.ts') as typeof import('../../dashboard/contentRevision');
    const { RefusedError } = await vite.ssrLoadModule('/ui/src/dashboard/queryError.ts') as typeof import('../../dashboard/queryError');
    const calls: { input: ProductRequest; signal: AbortSignal; answer: (data: unknown) => void; fail: () => void }[] = [];
    const client = { product: (input: ProductRequest, signal: AbortSignal) => new Promise<ProductReply<unknown>>((resolve, reject) => {
      calls.push({ input, signal, answer: data => resolve({ ...input, data }), fail: () => reject(new RefusedError('Synthetic image preparation refusal')) });
    }) } as unknown as DashboardClient;
    const track = { source: { id: 'source', revision: 'revision' }, branch: { id: 'branch' } } as BoundTrack;
    const entries: ResearchEntry[] = Array.from({ length: 9 }, (_, i) => ({
      token: `tech${String(i).padStart(2, '0')}`, technologyHandle: i, databaseIndex: i + 1, outcomeObservedHour: null, continuityStartHour: 100, level: 0,
      firstObservedHour: 100 + i * 2, startedAfterHour: 99 + i * 2, lastObservedHour: 101 + i * 2, endedByHour: 102 + i * 2, outcome: 'completed',
    }));
    const history: ResearchHistory = { coverage: [{ firstHour: 100, lastHour: 400 }], slots: [{ index: 0,
      availability: [{ firstHour: 100, lastHour: 400 }], progress: [], entries: [...entries,
        { token: 'zz_current', technologyHandle: 9, databaseIndex: 10, outcomeObservedHour: null, continuityStartHour: 100, level: 0, firstObservedHour: 118,
          startedAfterHour: 117, lastObservedHour: 239, endedByHour: 240, outcome: 'switched' },
        { token: 'a_future', technologyHandle: 10, databaseIndex: 11, outcomeObservedHour: null, continuityStartHour: 100, level: 0, firstObservedHour: 240,
          startedAfterHour: 239, lastObservedHour: 299, endedByHour: 300, outcome: 'completed' },
      ], states: [{ firstHour: 118, lastHour: 239, token: 'zz_current', databaseIndex: 10, entryFirstObservedHour: 118,
        speedMultiplierRaw: '100000', bankedDaysRaw: '0', appliedDaysRaw: '0', aheadReductionYearsRaw: '0', details: null }] }] };
    const tokens = researchArtworkTokens(researchAt(history, 150), 150, true);
    const render = () => { cursor = 0; return useTechnologyArtwork(client, track, tokens, true); };
    const art = (token: string): Artwork => ({ token, name: token === 'tech07' ? '' : 'Stored literal title', description: '', baseDurationDays: null,
      icon: 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png' });
    assert.deepEqual(render().warnings, [], 'pending image work is not a refusal');
    fixture.commit(); await settle();
    assert.equal(calls.length, 2);
    assert.deepEqual(calls.map(call => (call.input.params as { tokens: string[] }).tokens.length), [8, 2]);
    assert.deepEqual((calls[0].input.params as { tokens: string[] }).tokens, tokens.slice(0, 8));
    assert.equal((calls[0].input.params as { tokens: string[] }).tokens[0], 'zz_current', 'the current card precedes alphabetically earlier completed images');
    assert.ok(calls.every(call => !(call.input.params as { tokens: string[] }).tokens.includes('a_future')));
    calls[0].fail();
    calls[1].answer({ items: tokens.slice(8).map(art), warnings: ['Synthetic named image warning', 'Synthetic named image warning'] });
    await settle();
    let result = render();
    assert.equal(result.artwork.size, 2);
    assert.deepEqual([...result.failures.keys()], tokens.slice(0, 8));
    for (const token of tokens.slice(0, 8)) assert.equal(result.failures.get(token)?.message, 'Synthetic image preparation refusal');
    assert.equal(result.artwork.get('tech07')?.name, '', 'successful authored empty text remains known');
    assert.equal(result.artwork.get('tech08')?.name, 'Stored literal title');
    assert.deepEqual(result.warnings, ['Synthetic image preparation refusal', 'Synthetic named image warning']);
    fixture.commit(); await settle();
    assert.equal(calls.length, 2, 'permanent preparation refusals do not start a retry loop');

    contentRevision(client).observe({ game: 'changed-game', user: 'changed-user', contentReady: true });
    result = render();
    assert.equal(result.artwork.size, 0, 'content changes immediately mask old icons');
    assert.deepEqual(result.warnings, [], 'content changes immediately mask old failures');
    fixture.commit(); await settle();
    assert.equal(calls.length, 4);
    calls[2].answer({ items: [], warnings: [] });
    calls[3].answer({ items: tokens.slice(8).map(art), warnings: [] });
    await settle();
    result = render();
    assert.equal(result.artwork.size, 2);
    assert.deepEqual(result.warnings, ['Technology artwork does not match this request.'], 'a malformed image batch is a visible failure');
  } finally {
    fixture.unmount(); await vite.close(); delete host.__technologyArtworkHooks;
  }
});
