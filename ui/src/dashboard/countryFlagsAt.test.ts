import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { MAX_NAME_HOURS, nameKey } from './countryFacts.ts';
import type { DashboardClient } from './client';

const names = new Map<string, string | null | symbol>([[nameKey(10, 'named'), 'Recorded name'],
  [nameKey(10, 'missing'), null], [nameKey(20, 'named'), 'Later name']]);
const flags = new Map<string, { url?: string; reason?: string } | null | symbol>([[nameKey(10, 'named'), { url: 'recorded-flag' }], [nameKey(20, 'named'), { url: 'later-flag' }]]);
const calls: { kind: string; args: unknown[] }[] = [];
const host = globalThis as typeof globalThis & {
  __labelFixture?: { names: typeof names; flags: typeof flags; calls: typeof calls };
};
const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
  server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'country-label-fixture', enforce: 'pre',
    transform(source, id) {
      if (id.endsWith('/dashboard/countryNamesAt.ts')) return source.slice(0, source.indexOf('export function useCountryFactsAt'))
        + `export function useCountryFactsRead(...args) {
          globalThis.__labelFixture.calls.push({kind: 'flags', args}); return globalThis.__labelFixture.flags;
        }
        export const useCountryFactsAt = useCountryFactsRead;
        export function useCountryNamesRead(...args) {
          globalThis.__labelFixture.calls.push({kind: 'names', args}); return globalThis.__labelFixture.names;
        }`;
    },
  }] });
const [{ FAILED_FACT }, { useCountryLabelsAt }] = await Promise.all([
  vite.ssrLoadModule('/ui/src/dashboard/countryFacts.ts'), vite.ssrLoadModule('/ui/src/dashboard/countryFlagsAt.ts'),
]).finally(() => vite.close()) as [typeof import('./countryFacts.ts'), typeof import('./countryFlagsAt.ts')];
names.set(nameKey(10, 'failed'), FAILED_FACT);
flags.set(nameKey(10, 'failed-flag'), FAILED_FACT);
flags.set(nameKey(10, 'missing-flag'), { reason: 'The content sprite could not be decoded.' });

test('dated labels forward the hour limit to both reads and distinguish pending, missing and failed names', () => {
  const client = {} as DashboardClient;
  const wanted = ['pending', 'named', 'missing', 'failed', 'failed-flag', 'missing-flag'].map(tag => ({ hour: 10, tag }));
  host.__labelFixture = { names, flags, calls };
  try {
    for (const limit of [undefined, 400]) {
      calls.length = 0;
      function Labels() {
        const { labelAt } = useCountryLabelsAt(client, undefined, wanted, true, limit);
        assert.deepEqual(labelAt(10)('pending'), { name: 'pending', flag: undefined, known: undefined, nameFailed: false, flagPending: true, flagReason: undefined });
        assert.deepEqual(labelAt(10)('missing'), { name: 'missing', flag: undefined, known: false, nameFailed: false, flagPending: true, flagReason: undefined });
        assert.deepEqual(labelAt(10)('failed'), { name: 'failed', flag: undefined, known: false, nameFailed: true, flagPending: true, flagReason: undefined });
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
        assert.equal(call.args[call.kind === 'names' ? 4 : 5], limit ?? MAX_NAME_HOURS);
      }
      assert.equal((calls[1].args[4] as { product: string }).product, 'country-flags');
    }
  } finally { delete host.__labelFixture; }
});
