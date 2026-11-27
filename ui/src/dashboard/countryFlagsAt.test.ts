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
  [nameKey(10, 'pending'), nameReadFrom(undefined, true)], [nameKey(10, 'missing'), nameReadFrom(null, true)],
  [nameKey(10, 'refused-a'), nameReadFrom({ reason: 'localisation A: scripted trigger unavailable' }, true)],
  [nameKey(10, 'refused-b'), nameReadFrom({ reason: 'localisation B: missing key' }, true)],
  [nameKey(20, 'named'), nameReadFrom('Later name', true)]]);
const flags = new Map<string, { url?: string; reason?: string } | null | Error>([[nameKey(10, 'named'), { url: 'recorded-flag' }], [nameKey(20, 'named'), { url: 'later-flag' }]]);
const calls: { kind: string; args: unknown[] }[] = [];
const host = globalThis as typeof globalThis & {
  __labelFixture?: { names: typeof names; flags: typeof flags; calls: typeof calls };
};
const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
  server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'country-label-fixture', enforce: 'pre',
    transform(source, id) {
      if (id.endsWith('/dashboard/countryNamesAt.ts')) return source.slice(0, source.indexOf('export function useCountryFactsRead'))
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
        assert.deepEqual(labelAt(10)('refused-a'), { name: 'Country name unavailable', flag: undefined, known: false, nameFailed: false, nameDetail: 'localisation A: scripted trigger unavailable', flagPending: true, flagReason: undefined });
        assert.deepEqual(labelAt(10)('refused-b'), { name: 'Country name unavailable', flag: undefined, known: false, nameFailed: false, nameDetail: 'localisation B: missing key', flagPending: true, flagReason: undefined });
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
        assert.equal(call.args.length, call.kind === 'names' ? 7 : 6);
        if (call.kind === 'names') assert.equal(call.args[4], limit ?? Infinity);
        assert.equal(call.args.at(-1), false);
      }
    }
  } finally { delete host.__labelFixture; }
});
