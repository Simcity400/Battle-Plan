import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { CountryLooks } from './countryLooks';
import type { FlagRead } from './countryFlagsAt';
import { FAILED_FACT } from './countryFacts.ts';

// Drive the real sampled hook. Transport leaves are replaced; the plan, identity
// guard, held-look merge, flag spec and current pending/failure semantics remain.
test('visible sampled flags retain backend reasons, clear current failures and never decode the whole country inventory', async () => {
  const state = { slots: [] as { value: unknown; deps?: readonly unknown[] }[], cursor: 0, content: '',
    flags: new Map<string, FlagRead | null | typeof FAILED_FACT>(), current: undefined as CountryLooks | undefined,
    requests: [] as { product: string; tags: string[]; hours: number[]; batch: number }[] };
  const host = globalThis as typeof globalThis & { __lookFlags?: typeof state;
    __lookHook?: (kind: string, ...args: unknown[]) => unknown };
  host.__lookFlags = state;
  host.__lookHook = (kind, ...args) => {
    const index = state.cursor++, before = state.slots[index];
    if (kind === 'useRef') return (state.slots[index] ??= { value: { current: args[0] } }).value;
    const [factory, deps] = args as [() => unknown, readonly unknown[]];
    if (before?.deps?.length === deps.length && before.deps.every((value, at) => Object.is(value, deps[at]))) return before.value;
    const value = factory(); state.slots[index] = { value, deps: [...deps] }; return value;
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'sampled-flag-status', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/countryLooks.ts')) return source.replace(/\b(useMemo|useRef)(?:<[\s\S]*?>)?\(/g,
          (_match, kind) => `globalThis.__lookHook(${JSON.stringify(kind)}, `);
        if (path.endsWith('/dashboard/contentRevision.ts')) return 'export const useContentRevision = () => globalThis.__lookFlags.content;';
        if (path.endsWith('/dashboard/countryNamesAt.ts')) return `
          export const nameKey = (hour, tag) => JSON.stringify([hour, tag]);
          export const useCountryFactsAt = () => new Map(); export const useCountryNamesAt = () => new Map();
          export const useCountryFactsRead = (_client, _track, wanted, _active, spec) => {
            const state = globalThis.__lookFlags;
            state.requests.push({ product: spec.product, tags: wanted.map(row => row.tag), hours: wanted.map(row => row.hour), batch: spec.perRequest });
            const styles = spec.product === 'country-styles' ? spec.extract({
              styles: wanted.map(row => ({ tag: row.tag, name: row.tag, color: [1, 2, 3] })),
              warnings: ['native_4999: Synthetic mod name refusal'],
            }) : null;
            return new Map(wanted.flatMap(row => {
              const value = styles ? styles.get(row.tag) : state.flags.get(row.tag);
              return value === undefined ? [] : [[nameKey(row.hour, row.tag), value]];
            }));
          };`;
      },
    }] });
  try {
    const { useCountryLooks } = await vite.ssrLoadModule('/ui/src/dashboard/countryLooks.ts') as typeof import('./countryLooks.ts');
    const { flagReadSpec } = await vite.ssrLoadModule('/ui/src/dashboard/countryFlagsAt.ts') as typeof import('./countryFlagsAt.ts');
    const failed: typeof FAILED_FACT = (await vite.ssrLoadModule('/ui/src/dashboard/countryFacts.ts') as typeof import('./countryFacts.ts')).FAILED_FACT;
    const missing = await flagReadSpec('small').extract({ flags: [], unavailable: [{ tag: 'native_4999', reason: 'Synthetic mod flag refusal' }] });
    assert.equal(missing.get('native_4999')?.reason, 'Synthetic mod flag refusal');
    const tags = Array.from({ length: 5000 }, (_, index) => `native_${index}`);
    const track = { source: { id: 'source', revision: '1' }, branch: { id: 'branch', firstHour: 100, lastHour: 3000, gaps: [] } };
    let client = {}, hour = 205;
    let visible: readonly string[] = ['native_0', 'native_4999'];
    const segmentAt = (tag: string, at: number) => tag === 'native_0' && at >= 830 ? 830 : 100;
    const Probe = () => { state.current = useCountryLooks(client as never, track as never, tags, hour, true, segmentAt, undefined, false, visible); return null; };
    const render = () => { state.cursor = 0; state.requests = []; renderToStaticMarkup(createElement(Probe)); return state.current!; };
    let look = render();
    assert.equal(state.requests.find(row => row.product === 'country-styles')!.tags.length, 5000);
    assert.equal(state.requests.find(row => row.product === 'country-styles')!.batch, 64);
    assert.deepEqual(look.warnings, ['native_4999: Synthetic mod name refusal']);
    const flags = state.requests.find(row => row.product === 'country-flags')!;
    assert.deepEqual(flags.tags, visible);
    assert.deepEqual(flags.hours, [100, 100]);
    assert.equal(flags.batch, 32);
    assert.equal(look.flagPending('native_0', hour), true);
    state.flags.set('native_0', { url: 'data:sampled-flag' });
    state.flags.set('native_4999', missing.get('native_4999')!);
    look = render();
    assert.equal(look.flag('native_0', hour), 'data:sampled-flag');
    assert.equal(look.flagReason('native_4999', hour), 'Synthetic mod flag refusal');
    assert.equal(look.flagPending('native_4999', hour), false);
    assert.equal(look.flagReason('native_4999', 820), undefined, 'canvas motion cannot borrow the prior sample date reason');
    hour = 820; state.flags.clear();
    look = render();
    assert.equal(look.flag('native_0', hour), 'data:sampled-flag', 'same identity may hold a pending month sample');
    assert.equal(look.flagPending('native_0', hour), true);
    assert.equal(look.flagReason('native_4999', hour), undefined, 'an earlier refusal does not become the current date reason');
    state.flags.set('native_0', failed); state.flags.set('native_4999', null);
    look = render();
    assert.equal(look.flag('native_0', hour), undefined);
    assert.equal(look.flagReason('native_0', hour), 'The flag request failed.');
    assert.equal(look.flagPending('native_0', hour), false);
    assert.equal(look.flagReason('native_4999', hour), 'No readable flag was returned for this country at this hour.');
    state.flags.clear(); hour = 840;
    look = render();
    assert.equal(state.requests.find(row => row.product === 'country-flags')!.hours[0], 830);
    assert.equal(look.flag('native_0', hour), undefined, 'replacement does not hold the previous identity flag');
    assert.equal(look.flagReason('native_0', hour), undefined);
    assert.equal(look.flagPending('native_0', hour), true);
    visible = ['native_4999']; render();
    assert.deepEqual(state.requests.find(row => row.product === 'country-flags')!.tags, visible);
    visible = []; render();
    assert.equal(state.requests.find(row => row.product === 'country-flags')!.tags.length, 0);
    visible = ['native_0']; hour = 205;
    for (const boundary of ['client', 'source', 'revision', 'branch', 'content'] as const) {
      state.flags.set('native_0', { url: 'data:old-scope' }); render(); state.flags.clear();
      if (boundary === 'client') client = {};
      if (boundary === 'source') track.source.id += '-changed';
      if (boundary === 'revision') track.source.revision += '-changed';
      if (boundary === 'branch') track.branch.id += '-changed';
      if (boundary === 'content') state.content += '-changed';
      look = render();
      assert.equal(look.flag('native_0', hour), undefined, boundary);
      assert.equal(look.flagReason('native_0', hour), undefined, boundary);
      assert.equal(look.flagPending('native_0', hour), true, boundary);
    }
  } finally { await vite.close(); delete host.__lookFlags; delete host.__lookHook; }
});
