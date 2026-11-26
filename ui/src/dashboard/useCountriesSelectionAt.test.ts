import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetRuntime } from './definitions';
import type { CountrySelection, WidgetSpec } from './model';
import type { CountryGroupReply } from '../generated/dashboard';
import { at } from './regionTime.ts';

const native = (firstHour: number, lastHour: number, countryId: number) => ({ firstHour, lastHour,
  slot: countryId === 10 ? 1 : countryId === 11 ? 2 : 3, countryId });
const catalogue = (): CountryGroupReply => ({ branchId: 'branch', humans: [{ tag: 'Y', firstHour: 120, lastHour: 127 }],
  factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [], warnings: [], unobserved: { humans: [], factions: [], wars: [], unattributed: [] },
  countryIdentities: [
    { tag: 'A', identities: [native(90, 99, 10)] },
    { tag: 'X', identities: [native(100, 109, 10), native(140, 160, 11)] },
    { tag: 'B', identities: [native(90, 139, 11)] },
    { tag: 'Y', identities: [native(100, 119, 99), native(120, 129, 10), native(131, 139, 10)] },
  ] });

type State = { groups?: CountryGroupReply; groupReads: { track?: string; active: boolean }[] };

async function fixture() {
  const state: State = { groupReads: [] };
  const globals = globalThis as typeof globalThis & { __holderCompanions?: State };
  globals.__holderCompanions = state;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'holder-companion-callers', enforce: 'pre',
      transform(_source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/useCountryGroups.ts')) return `export const useCountryGroups = (_runtime, track, active = true) => {
          const state = globalThis.__holderCompanions; state.groupReads.push({ track: track?.id, active });
          return { data: track && active ? state.groups : undefined };
        };`;
      },
    }] });
  const { useCountriesSelectionAt } = await vite.ssrLoadModule('/ui/src/dashboard/useCountriesSelectionAt.ts') as typeof import('./useCountriesSelectionAt');
  const track = { id: 'main', selectedSlot: 1, axisOffset: 0, branch: { id: 'branch', firstHour: 90, lastHour: 160 },
    source: { id: 'source', revision: '1', metadata: { countries: [] } } };
  const runtime = { tracks: [track], client: {}, clock: { getHour: () => 160 } } as unknown as WidgetRuntime;
  function read(selection: CountrySelection, hour: number) {
    const widget: WidgetSpec = { id: 'widget', kind: 'generals', title: 'Generals', configVersion: 1,
      trackIds: ['main'], countries: selection, time: { mode: 'follow' } };
    let result!: ReturnType<typeof useCountriesSelectionAt>;
    function Probe() {
      result = useCountriesSelectionAt(widget, runtime, true, at(hour));
      return null;
    }
    renderToStaticMarkup(createElement(Probe));
    return result;
  }
  return { state, read,
    async close() { await vite.close(); delete globals.__holderCompanions; } };
}

test('actual selection hook loads manual identities and keeps literal request roots through alias reuse', async () => {
  const view = await fixture();
  try {
    const manual: CountrySelection = { kind: 'tags', tags: ['X'] };
    const pending = view.read(manual, 125);
    assert.deepEqual(view.state.groupReads.at(-1), { track: 'main', active: true });
    assert.equal(pending.countries, undefined);
    assert.equal(pending.requestTags, undefined, 'pending manual picks never become an all-world request');
    view.state.groups = catalogue();
    const selected = view.read(manual, 125);
    assert.deepEqual(new Set(selected.countries), new Set(['B', 'Y']));
    assert.deepEqual(selected.requestTags, ['X'], 'alias Y is never a new root that admits its unrelated holder');
    assert.deepEqual(view.read(manual, 105).countries, new Set(['X', 'B']));
    assert.deepEqual(view.read(manual, 130).countries, new Set(['B']), 'native gaps acquire no holder');
    assert.deepEqual(view.read(manual, 95).countries, new Set(['A', 'B']));
    assert.deepEqual(view.read({ kind: 'selected' }, 95).countries, new Set(['A']), 'the selected slot has one holder');
    const missing = view.read({ kind: 'tags', tags: ['unknown'] }, 125);
    assert.equal(missing.requestTags, undefined);
    assert.match(missing.selectionReason!, /Selected countries are unavailable in this campaign\./);
    const all = view.read({ kind: 'all' }, 125);
    assert.equal(all.countries, null); assert.equal(all.requestTags, null);
    assert.equal(view.state.groupReads.at(-1)!.track, undefined);
    const group: CountrySelection = { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false };
    assert.deepEqual(view.read(group, 125).countries, new Set(['Y']));
    assert.equal(view.read(group, 128).countries, undefined, 'hourly groups keep their own membership dates');
  } finally { await view.close(); }
});
