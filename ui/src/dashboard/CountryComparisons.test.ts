import assert from 'node:assert/strict';
import test from 'node:test';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

test('country comparison headers retain neutral missing flags and request only mounted panels at 5,000 countries', async () => {
  const state = { tags: Array.from({ length: 5000 }, (_, index) => `native-${index}`), wanted: [] as { tag: string; hour: number }[] };
  const host = globalThis as typeof globalThis & { __comparisonHeaders?: typeof state };
  host.__comparisonHeaders = state;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'comparison-header-flags', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/CountryComparisons.tsx') || path.endsWith('/dashboard/useDiagnostics.ts')) return source.replace(/import \{ [^}]+ \} from 'react';/,
          'const useEffect = () => {}, useMemo = build => build(), useRef = current => ({ current }), useState = value => [value, () => {}];');
        if (path.endsWith('/dashboard/playback.ts')) return 'export const usePlaybackHour = clock => clock.getHour();';
        if (path.endsWith('/dashboard/comparisonCountries.ts')) return `export const useComparisonCountries = () => ({
          countries: { tags: globalThis.__comparisonHeaders.tags }, retry() {}, pending: false });`;
        if (path.endsWith('/dashboard/countryFlagsAt.ts')) return `export const useCountryLabelsAt = (client, track, wanted) => {
          globalThis.__comparisonHeaders.wanted = wanted;
          return { labelAt: () => tag => ({ name: tag, flagReason: 'Synthetic dated flag missing', flagPending: false }) };
        };`;
      },
    }] });
  try {
    const { default: CountryComparisons } = await vite.ssrLoadModule('/ui/src/dashboard/CountryComparisons.tsx');
    const html = renderToStaticMarkup(CountryComparisons({ component: () => null, active: true, onDiagnostics() {},
      widget: { time: { mode: 'campaign' } }, runtime: { clock: { getHour: () => 0 } } }));
    assert.ok(state.wanted.length > 0 && state.wanted.length < 10);
    assert.equal((html.match(/country-flag-unavailable/g) ?? []).length, state.wanted.length);
    assert.match(html, /Flag unavailable/);
    assert.doesNotMatch(html, /Synthetic dated flag missing/);
    assert.doesNotMatch(html, /<img/);
  } finally { delete host.__comparisonHeaders; await vite.close(); }
});
