import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { nameKey } from './countryFacts.ts';
import type { BoundTrack } from './model';
import { nameReadFrom, type NameRead } from './nameReads.ts';
import type { DashboardClient } from './client';

const names = new Map<string, NameRead>([[nameKey(10, 'named'), nameReadFrom('Recorded name', true)],
  [nameKey(10, 'pending'), nameReadFrom(undefined, true)], [nameKey(10, 'missing'), nameReadFrom(null, true)], [nameKey(20, 'named'), nameReadFrom('Later name', true)]]);
const flags = new Map<string, { url?: string; reason?: string } | null | Error>([[nameKey(10, 'named'), { url: 'recorded-flag' }], [nameKey(20, 'named'), { url: 'later-flag' }]]);
const calls: { kind: string; args: unknown[] }[] = [];
const host = globalThis as typeof globalThis & {
  __labelFixture?: { names: typeof names; flags: typeof flags; calls: typeof calls };
};
const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
  server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'country-label-fixture', enforce: 'pre',
    transform(source, id) {
      if (id.endsWith('/dashboard/countryNamesAt.ts')) return source.slice(0, source.indexOf('export function useCountryFactsAt'))
        + `export function useCountryNamesAt(...args) {
          globalThis.__labelFixture.calls.push({kind: 'names', args}); return globalThis.__labelFixture.names;
        }`;
      if (id.endsWith('/dashboard/countryFlags.ts')) return `export function useCountryFlagsRead(...args) {
          globalThis.__labelFixture.calls.push({kind: 'flags', args}); return globalThis.__labelFixture.flags;
        }`;
    },
  }] });
const [{ FAILED_FACT }, { useCountryLabelsAt }] = await Promise.all([
  vite.ssrLoadModule('/ui/src/dashboard/countryFacts.ts'), vite.ssrLoadModule('/ui/src/dashboard/countryFlagsAt.ts'),
]).finally(() => vite.close()) as [typeof import('./countryFacts.ts'), typeof import('./countryFlagsAt.ts')];
names.set(nameKey(10, 'failed'), nameReadFrom(FAILED_FACT, true));
flags.set(nameKey(10, 'failed-flag'), FAILED_FACT);
flags.set(nameKey(10, 'missing-flag'), { reason: 'The content sprite could not be decoded.' });

test('dated labels forward the hour limit to the name read and distinguish pending, missing and failed names', () => {
  const client = {} as DashboardClient;
  const wanted = ['pending', 'named', 'missing', 'failed', 'failed-flag', 'missing-flag'].map(tag => ({ hour: 10, tag }));
  const track = { branch: { firstHour: 0, lastHour: 30 } } as BoundTrack;
  for (const tag of ['failed-flag', 'missing-flag']) names.set(nameKey(10, tag), nameReadFrom(undefined, true));
  host.__labelFixture = { names, flags, calls };
  try {
    for (const limit of [undefined, 400]) {
      calls.length = 0;
      function Labels() {
        const { labelAt } = useCountryLabelsAt(client, track, wanted, true, limit);
        assert.deepEqual(labelAt(10)('pending'), { name: 'Loading name…', flag: undefined, known: undefined, nameFailed: false, flagPending: true, flagReason: undefined });
        assert.deepEqual(labelAt(10)('missing'), { name: 'Country name unavailable', flag: undefined, known: false, nameFailed: false, flagPending: true, flagReason: undefined });
        assert.deepEqual(labelAt(10)('failed'), { name: 'Couldn’t load country name.', flag: undefined, known: false, nameFailed: true, nameDetail: FAILED_FACT.message, nameFailure: FAILED_FACT, flagPending: true, flagReason: undefined });
        assert.deepEqual(labelAt(10)('named'), { name: 'Recorded name', flag: 'recorded-flag', known: true, nameFailed: false, flagPending: false, flagReason: undefined });
        assert.equal(labelAt(10)('failed-flag').flagReason, 'The flag request failed.');
        assert.equal(labelAt(10)('failed-flag').flagPending, false);
        assert.equal(labelAt(10)('missing-flag').flagReason, 'The content sprite could not be decoded.');
        assert.equal(labelAt(20)('named').name, 'Later name');
        return null;
      }
      renderToStaticMarkup(createElement(Labels));
      assert.deepEqual(calls.map(call => call.kind), ['names', 'flags']);
      for (const call of calls) {
        assert.equal(call.args[0], client);
        assert.equal(call.args[2], wanted);
        assert.equal(call.args[3], true);
        assert.equal(call.args.length, 5);
        if (call.kind === 'names') assert.equal(call.args[4], limit ?? Infinity);
        else assert.equal(call.args[4], 'panel');
      }
    }
  } finally { delete host.__labelFixture; }
});

test('dated labels never borrow another hour’s cached name or flag while the requested hour is still pending', () => {
  const client = {} as DashboardClient;
  const track = { branch: { firstHour: 0, lastHour: 30 } } as BoundTrack;
  const localNames = new Map<string, NameRead>([
    [nameKey(20, 'GER'), nameReadFrom('Later name', true)],
    [nameKey(10, 'GER'), nameReadFrom(undefined, true)],
  ]);
  const localFlags = new Map<string, { url?: string; reason?: string } | null | Error>([
    [nameKey(20, 'GER'), { url: 'later-flag' }],
  ]);
  host.__labelFixture = { names: localNames, flags: localFlags, calls };
  try {
    function Labels() {
      const { labelAt } = useCountryLabelsAt(client, track, [{ hour: 20, tag: 'GER' }, { hour: 10, tag: 'GER' }], true);
      const later = labelAt(20)('GER');
      assert.equal(later.name, 'Later name');
      assert.equal(later.flag, 'later-flag');
      const earlier = labelAt(10)('GER');
      assert.deepEqual(earlier, { name: 'Loading name…', flag: undefined, known: undefined, nameFailed: false, flagPending: true, flagReason: undefined });
      return null;
    }
    renderToStaticMarkup(createElement(Labels));
  } finally { delete host.__labelFixture; }
});

test('useFlagReads never borrows another hour’s drawn flag while a different run’s flag is still drawing', async () => {
  const slots = new Map<number, unknown>(), effects = new Map<number, { deps?: readonly unknown[]; run: () => void | (() => void); cleanup?: () => void }>(),
    pending = new Map<number, { deps?: readonly unknown[]; run: () => void | (() => void); cleanup?: () => void }>();
  let cursor = 0;
  const same = (a?: readonly unknown[], b?: readonly unknown[]) => a && b && a.length === b.length && a.every((item, i) => Object.is(item, b[i]));
  const hookState = {
    begin() { cursor = 0; },
    useRef(initial: unknown) { const index = cursor++; if (!slots.has(index)) slots.set(index, { current: initial }); return slots.get(index); },
    useState(initial: unknown) {
      const index = cursor++; if (!slots.has(index)) slots.set(index, initial);
      return [slots.get(index), (next: unknown) => { slots.set(index, typeof next === 'function' ? next(slots.get(index)) : next); }];
    },
    useMemo(make: () => unknown, deps: readonly unknown[]) {
      const index = cursor++, before = slots.get(index) as { deps: readonly unknown[]; value: unknown } | undefined;
      if (!before || !same(before.deps, deps)) slots.set(index, { deps, value: make() });
      return (slots.get(index) as { value: unknown }).value;
    },
    useEffect(run: () => void | (() => void), deps?: readonly unknown[]) {
      const index = cursor++; if (!same(effects.get(index)?.deps, deps)) pending.set(index, { run, deps });
    },
    commit() {
      for (const [index, effect] of pending) {
        effects.get(index)?.cleanup?.(); const cleanup = effect.run();
        effect.cleanup = typeof cleanup === 'function' ? cleanup : undefined; effects.set(index, effect);
      }
      pending.clear();
    },
    dispose() { for (const effect of effects.values()) effect.cleanup?.(); },
  };
  const flagHost = globalThis as typeof globalThis & { __flagNoBorrowHooks?: typeof hookState };
  flagHost.__flagNoBorrowHooks = hookState;
  const server = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'flag-no-borrow-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/contentRevision.ts')) return 'export const useContentRevision = () => 0;';
        if (path.endsWith('/dashboard/countryFlags.ts'))
          return source.replace(/import \{ ([^}]+) \} from 'react';/, (_match, names: string) => names.split(',').map(name =>
            `const ${name.trim()} = (...args) => globalThis.__flagNoBorrowHooks.${name.trim()}(...args);`).join('\n'));
      } }] });
  try {
    const { useFlagReads, flagReadKey } = await server.ssrLoadModule('/ui/src/dashboard/countryFlags.ts') as typeof import('./countryFlags.ts');
    const recording = { id: 'flag-src', revision: '1' } as BoundTrack['source'];
    const reply = {
      branchId: '1', known: [[0, 300]], names: ['Earlier', 'Later'], reasons: [], warnings: [],
      flags: [
        { keys: ['GER_early', null, null], reasons: [null, null, null] },
        { keys: ['GER_late', null, null], reasons: [null, null, null] },
      ],
      countries: [{ tag: 'GER', runs: [[0, 0, -1, -1, 0], [150, 1, -1, -1, 1]] }],
    };
    const drawResolvers: (() => void)[] = [];
    const flagClient = {
      product: (request: { sourceId: string; revision: string; product: string; params: { flags?: { keys: (string | null)[] }[] } }) => {
        if (request.product === 'country-facts') return Promise.resolve({ ...request, data: reply });
        if (request.product === 'country-flags') {
          const choices = request.params.flags ?? [];
          if (choices.every(choice => choice.keys[0] === 'GER_late')) {
            return Promise.resolve({ ...request, data: { flags: [{ flag: 0, width: 10, height: 6, png: 'data:later-flag' }], unavailable: [] } });
          }
          return new Promise(resolve => {
            drawResolvers.push(() => resolve({ ...request, data: { flags: [{ flag: 0, width: 10, height: 6, png: 'data:early-flag' }], unavailable: [] } }));
          });
        }
        return Promise.resolve({ ...request, data: {} });
      },
    } as unknown as DashboardClient;

    const flush = async () => { for (let i = 0; i < 4; i++) await new Promise<void>(resolve => setImmediate(resolve)); };
    hookState.begin(); useFlagReads(flagClient, recording, [{ branchId: '1', hour: 200, tag: 'GER' }], true, 'panel'); hookState.commit();
    await flush();
    hookState.begin(); useFlagReads(flagClient, recording, [{ branchId: '1', hour: 200, tag: 'GER' }], true, 'panel'); hookState.commit();
    await flush();
    hookState.begin();
    const at200 = useFlagReads(flagClient, recording, [{ branchId: '1', hour: 200, tag: 'GER' }], true, 'panel');
    hookState.commit();
    assert.deepEqual(at200.get(flagReadKey({ branchId: '1', hour: 200, tag: 'GER' })), { url: 'data:later-flag', image: { flag: 0, width: 10, height: 6, png: 'data:later-flag' } });

    hookState.begin();
    const at100Pending = useFlagReads(flagClient, recording, [{ branchId: '1', hour: 100, tag: 'GER' }], true, 'panel');
    hookState.commit();
    assert.equal(at100Pending.get(flagReadKey({ branchId: '1', hour: 100, tag: 'GER' })), undefined, 'must not borrow hour 200 flag for pending hour 100');

    drawResolvers.shift()?.();
    await flush();
    hookState.begin();
    const at100Settled = useFlagReads(flagClient, recording, [{ branchId: '1', hour: 100, tag: 'GER' }], true, 'panel');
    hookState.commit();
    assert.equal((at100Settled.get(flagReadKey({ branchId: '1', hour: 100, tag: 'GER' })) as { url?: string } | undefined)?.url, 'data:early-flag');
    hookState.dispose();
  } finally { delete flagHost.__flagNoBorrowHooks; await server.close(); }
});
