import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { ProductRequest } from './client';
import type { WidgetRuntime } from './definitions';
import type { CountrySelection, WidgetSpec } from './model';
import type { CountryGroupReply, PoliticsLedgerReply } from '../generated/dashboard';
import { spendingGaps } from './politicsSpending.ts';

const native = (firstHour: number, lastHour: number, countryId: number) => ({ firstHour, lastHour,
  slot: countryId === 10 ? 1 : countryId === 11 ? 2 : 3, countryId });
const catalogue = (): CountryGroupReply => ({ branchId: 'branch', humans: [{ tag: 'Y', firstHour: 120, lastHour: 127 }],
  factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [], warnings: [], unobserved: { humans: [], factions: [], wars: [] },
  countryIdentities: [
    { tag: 'A', identities: [native(90, 99, 10)] },
    { tag: 'X', identities: [native(100, 109, 10), native(140, 160, 11)] },
    { tag: 'B', identities: [native(90, 139, 11)] },
    { tag: 'Y', identities: [native(100, 119, 99), native(120, 129, 10), native(131, 139, 10)] },
  ] });

type State = { groups?: CountryGroupReply; error?: string; groupReads: { track?: string; active: boolean }[];
  requests: readonly ProductRequest<{ tag: string }>[]; groupRetries: number; ledgerRetries: number };

async function fixture() {
  const state: State = { groupReads: [], requests: [], groupRetries: 0, ledgerRetries: 0 };
  const globals = globalThis as typeof globalThis & { __holderCompanions?: State };
  globals.__holderCompanions = state;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'holder-companion-callers', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/useCountryGroups.ts')) return `export const useCountryGroups = (_runtime, track, active = true) => {
          const state = globalThis.__holderCompanions; state.groupReads.push({ track: track?.id, active });
          return { data: track && active ? state.groups : undefined, error: state.error, retry() { state.groupRetries++; } };
        };`;
        if (path.endsWith('/dashboard/products.ts')) return `export const useProducts = (_client, requests) => {
          const state = globalThis.__holderCompanions; state.requests = requests;
          return { data: [], retry() { state.ledgerRetries++; } };
        };`;
        if (path.endsWith('/dashboard/useIdeaTooltips.ts')) return 'export const useIdeaTooltips = () => ({});';
        if (path.endsWith('/dashboard/useCountriesSelectionAt.ts') || path.endsWith('/dashboard/usePoliticsSpending.ts')) {
          return source.replace(/\buseMemo(?:<[^;\n]*>)?\(/g, '((factory) => factory())(');
        }
      },
    }] });
  const countries = await vite.ssrLoadModule('/ui/src/dashboard/useCountriesSelectionAt.ts') as typeof import('./useCountriesSelectionAt');
  const politics = await vite.ssrLoadModule('/ui/src/dashboard/usePoliticsSpending.ts') as typeof import('./usePoliticsSpending');
  const track = { id: 'main', selectedTag: 'X', axisOffset: 0, branch: { id: 'branch', firstHour: 90, lastHour: 160 },
    source: { id: 'source', revision: '1', metadata: { countries: [] } } };
  const runtime = { tracks: [track], client: {} } as unknown as WidgetRuntime;
  const widget = (selection: CountrySelection): WidgetSpec => ({ id: 'widget', kind: 'generals', title: 'Generals', configVersion: 1,
    trackIds: ['main'], countries: selection, time: { mode: 'follow' } });
  return { state, runtime, widget, countries: countries.useCountriesSelectionAt, politics: politics.usePoliticsSpending,
    async close() { await vite.close(); delete globals.__holderCompanions; } };
}

test('actual selection hook loads manual identities and keeps literal request roots through alias reuse', async () => {
  const view = await fixture();
  try {
    const widget = view.widget({ kind: 'tags', tags: ['X'] });
    const pending = view.countries(widget, view.runtime, 125, true);
    assert.deepEqual(view.state.groupReads.at(-1), { track: 'main', active: true });
    assert.equal(pending.countries, undefined);
    assert.equal(pending.requestTags, undefined, 'pending manual picks never become an all-world request');
    view.state.groups = catalogue();
    const selected = view.countries(widget, view.runtime, 125, true);
    assert.deepEqual(new Set(selected.countries), new Set(['B', 'Y']));
    assert.deepEqual(selected.requestTags, ['X'], 'alias Y is never a new root that admits its unrelated holder');
    assert.deepEqual(new Set(view.countries(widget, view.runtime, 105, true).countries), new Set(['X', 'B']));
    assert.deepEqual(new Set(view.countries(widget, view.runtime, 130, true).countries), new Set(['B']), 'native gaps acquire no holder');
    assert.deepEqual(new Set(view.countries(view.widget({ kind: 'selected' }), view.runtime, 95, true).countries), new Set(['A', 'B']));
    const missing = view.countries(view.widget({ kind: 'tags', tags: ['unknown'] }), view.runtime, 125, true);
    assert.equal(missing.requestTags, undefined);
    assert.match(missing.selectionReason!, /Selected countries are unavailable in this campaign\./);
    const all = view.countries(view.widget({ kind: 'all' }), view.runtime, 125, true);
    assert.equal(all.countries, null); assert.equal(all.requestTags, null);
    assert.equal(view.state.groupReads.at(-1)!.track, undefined);
    const group = view.widget({ kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false });
    assert.deepEqual(new Set(view.countries(group, view.runtime, 125, true).countries), new Set(['Y']));
    assert.equal(view.countries(group, view.runtime, 128, true).countries, undefined, 'hourly groups keep their own membership dates');
  } finally { await view.close(); }
});

test('actual spending hook fetches dated alias ledgers without broadening rows or coverage', async () => {
  const view = await fixture();
  try {
    const widget = view.widget({ kind: 'tags', tags: ['X'] });
    const pending = view.politics(widget, view.runtime, true);
    assert.equal(pending.ready, false); assert.deepEqual(view.state.requests, []);
    assert.equal(view.state.groupReads.at(-1)!.active, true);
    view.state.groups = catalogue();
    const selected = view.politics(widget, view.runtime, true);
    assert.equal(selected.ready, true);
    assert.equal(selected.selection.requireRecordedHolder, true);
    const requests = (): State['requests'] => view.state.requests;
    assert.deepEqual(new Set(requests().map(request => request.params.tag)), new Set(['A', 'B', 'X', 'Y']));
    assert.equal(selected.selection.matches('Y', 105), false, 'the unrelated former alias holder is excluded');
    assert.equal(selected.selection.matches('Y', 125), true);
    assert.equal(selected.selection.matches('Y', 130), false, 'missing native identity is unknown');
    assert.equal(selected.selection.matches('B', 105), true, 'earlier aliases of a later X holder remain selected');
    const ledger = { tag: 'Y', spendingCoverage: [{ readHour: 139, sinceHour: 100, throughHour: 139,
      political: false, command: false, reason: 'The stream was not recorded' }] } as unknown as PoliticsLedgerReply;
    assert.deepEqual(spendingGaps([ledger], 'Political', { from: null, hour: 160 }, selected.selection)
      .map(gap => [gap.from, gap.through]), [[120, 129], [131, 139]], 'coverage is clipped to exact selected native spans');
    const group = view.widget({ kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false });
    const grouped = view.politics(group, view.runtime, true);
    assert.deepEqual(grouped.tags, ['Y']);
    assert.equal(grouped.selection.requireRecordedHolder, false, 'a known empty smart group has no manual holder expectation');
    assert.equal(grouped.selection.matches('Y', 119), false); assert.equal(grouped.selection.matches('Y', 125), true);
    assert.equal(grouped.selection.matches('Y', 128), false);
    assert.deepEqual(spendingGaps([ledger], 'Political', { from: null, hour: 160 }, grouped.selection)
      .map(gap => [gap.from, gap.through]), [[120, 127]]);
    const ever = view.politics(view.widget({ kind: 'group', group: { type: 'humans' }, membership: 'ever', aggregate: false }), view.runtime, true);
    assert.deepEqual(new Set(ever.tags), new Set(['A', 'X', 'Y']), 'ever counts only the native holder inside recorded membership dates');
    assert.equal(ever.selection.matches('Y', 105), false); assert.equal(ever.selection.matches('X', 150), false);
    view.state.groups = { ...catalogue(), humans: [{ tag: 'X', firstHour: 140, lastHour: 160 }] };
    const end = view.politics(view.widget({ kind: 'group', group: { type: 'humans' }, membership: 'end', aggregate: false }), view.runtime, true);
    assert.deepEqual(new Set(end.tags), new Set(['B', 'X']));
    assert.equal(end.selection.matches('B', 105), true); assert.equal(end.selection.matches('Y', 125), false);
    view.state.groups = catalogue();
    view.state.error = 'Native catalogue could not be read';
    const failed = view.politics(widget, view.runtime, true);
    assert.equal(failed.error, view.state.error); failed.retry();
    assert.equal(view.state.groupRetries, 1); assert.equal(view.state.ledgerRetries, 1);
    const missing = view.politics(view.widget({ kind: 'tags', tags: ['unknown'] }), view.runtime, true);
    assert.deepEqual(missing.tags, []); assert.deepEqual(view.state.requests, []);
    assert.match(missing.reason!, /Selected countries are unavailable in this campaign\./);
  } finally { await view.close(); }
});
