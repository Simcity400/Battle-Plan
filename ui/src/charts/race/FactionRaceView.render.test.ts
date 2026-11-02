import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { DrillPart } from '../drill/DrillParts';

async function server(replacements: Record<string, string>, transform?: (source: string, path: string) => string | undefined) {
  return createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'shared-race-presentation', enforce: 'pre',
      transform(source, id) { const path = id.replaceAll('\\', '/'); return Object.entries(replacements).find(([suffix]) => path.endsWith(suffix))?.[1] ?? transform?.(source, path); },
    }] });
}

test('rendered faction members distinguish duplicate names and keep unknown rows exact', async () => {
  const state = { parts: undefined as { parts: readonly DrillPart[] } | undefined };
  const host = globalThis as typeof globalThis & { __memberPresentation?: typeof state }; host.__memberPresentation = state;
  const vite = await server({
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/dashboard/countryNamesAt.ts': 'export const useCountryFactsRead = () => new Map();',
    '/dashboard/useCountriesAt.ts': 'export const STYLES = 0;',
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = () => ({labelAt: () => tag => ({name: tag === 'C' ? 'Distinct' : 'Same', flag: undefined})});`,
    '/dashboard/playback.ts': 'export const usePlaybackMotion = () => 0;',
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({muted: '#888', surface: '#000', countryColours: 'fixed'});`,
    '/charts/race/factionPartColours.ts': `export const factionColourPlan = () => ({wanted: [], selection: new Map(), maxHours: 0});
      export const factionColourKey = () => null; export const factionCountryColours = () => new Map(); export const heldFactionColours = () => new Map();`,
    '/charts/TimeChart.tsx': 'export default () => null;',
    '/charts/drill/DrillParts.tsx': 'export default props => {globalThis.__memberPresentation.parts = props; return null;};',
  });
  try {
    const { default: View } = await vite.ssrLoadModule('/ui/src/charts/race/FactionPartsView.tsx');
    const parts = ['A', 'B', 'C'].map((tag, index) => ({id: tag, runs: [{firstHour: 0, lastHour: 10, raw: index === 1 ? null : 10n, value: index === 1 ? 0 : 10}],
      counted: [{firstHour: 0, lastHour: 10}], witnesses: [{firstHour: 0, lastHour: 10, tag}]}));
    const html = renderToStaticMarkup(createElement(View, { model: {parts, census: [], total: [], remainder: [], unidentified: 1},
      runtime: {client: {}, clock: {}}, track: {source: {id: 'source'}, branch: {id: 'branch'}}, hour: 5, following: false,
      extent: [0, 10], active: true, pickOwner: {}, onPick() {}, label: 'Faction', formatValue: String }));
    assert.deepEqual(state.parts!.parts.map(part => part.label), ['Same (A)', 'Same (B)', 'Distinct']);
    assert.equal(state.parts!.parts[1].raw, null);
    assert.equal(state.parts!.parts[1].text, 'Not recorded');
    assert.match(html, /Faction total unavailable on/);
    assert.match(html, /Some member values are unavailable/);
    assert.doesNotMatch(html, /map colours|native identity|Known member values/);
  } finally { await vite.close(); delete host.__memberPresentation; }
});

test('faction races request and display numeric totals during a colour failure, including a 5000-country catalogue', async () => {
  const state = { chart: undefined as {entityLooks: {color: (id: string) => string}; ranked: unknown[]; onPick: (hour: number, id: string) => void} | undefined,
    requested: 0, cursor: 0, slots: [] as unknown[], hour: 10, pins: [] as import('../../dashboard/model').WidgetSpec[],
    detail: undefined as {hour: number; following: boolean} | undefined,
    control: undefined as {onPause: () => void; onFollow: () => void} | undefined,
    host: undefined as {onPause: () => void; onFollow: () => void; onPin: () => void} | undefined,
    looks: {loading: false, error: new Error('Synthetic colour failure') as Error | undefined, factions: [] as {id: string}[], names: new Map(), colours: new Map<string, string>(), warnings: []} };
  const hooks = {
    useMemo(build: () => unknown, deps: readonly unknown[]) {
      const index = state.cursor++, before = state.slots[index] as { deps: readonly unknown[]; value: unknown } | undefined;
      if (before && before.deps.length === deps.length && deps.every((value, i) => Object.is(value, before.deps[i]))) return before.value;
      const value = build(); state.slots[index] = { deps, value }; return value;
    },
    useRef(initial: unknown) { const index = state.cursor++; return state.slots[index] ??= { current: initial }; },
    useState(initial: unknown) { const index = state.cursor++; if (!(index in state.slots)) state.slots[index] = initial;
      return [state.slots[index], (next: unknown) => { state.slots[index] = typeof next === 'function' ? next(state.slots[index]) : next; }]; },
  };
  const host = globalThis as typeof globalThis & { __factionPresentation?: typeof state; __factionRenderHooks?: typeof hooks };
  host.__factionPresentation = state; host.__factionRenderHooks = hooks;
  const vite = await server({
    '/dashboard/FactionColours.tsx': `export const useFactionColours = () => globalThis.__factionPresentation.looks;`,
    '/dashboard/factionTotals.ts': `export const factionTotalsRequest = (branchId, _selection, source, top) => {globalThis.__factionPresentation.requested = top; return {branchId, source, top};};
      export const factionTotalsModel = () => ({series: new Map()}); export const factionLines = () => Array.from({length: 5000}, (_, index) => ({tag: 'faction-' + index, runs: [{firstHour: 0, lastHour: 10, value: 42}]}));
      export const factionLabelAt = () => 'Faction'; export const factionNameAt = () => 'Faction'; export const factionMemberLabelLimit = () => 1;
      export const factionTooltipForms = () => []; export const factionVisibleMemberTags = () => []; export const validateFactionTotalsReply = value => value;`,
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/dashboard/preparedFactionTotals.ts': `const data = {series: [{id: '1', membership: [{firstHour: 0, lastHour: 100}]}], warnings: [], unit: 'Count', unavailableReason: null};
      export const usePreparedFactionTotals = (client, track, params) => ({data: params ? data : undefined, lineSummaries: new Map(), navigation: {},
        owner: globalThis.__factionRenderHooks.useMemo(() => ({}), [client, JSON.stringify(params)]), onSummaryView() {}, onReachHour() {}});`,
    '/charts/useChartTokens.ts': `export const useViewportHeight = () => 300; export const useChartTokens = () => ({muted: '#888'});`,
    '/charts/race/CountryRaceView.tsx': 'export default props => {globalThis.__factionPresentation.chart = props; return null;};',
    '/charts/race/factionParts.ts': `export * from '/ui/src/charts/race/factionParts.ts?actual'; export const factionNumbers = reply => reply; export const factionParts = () => ({});`,
    '/charts/race/FactionPartsView.tsx': 'export default props => {globalThis.__factionPresentation.detail = props; return null;};',
    '/charts/drill/FollowControl.tsx': `import Actual from '/ui/src/charts/drill/FollowControl.tsx?actual'; import {createElement} from 'react';
      export default props => {globalThis.__factionPresentation.control = props; return createElement(Actual, props);};`,
    '/charts/drill/DrillHost.tsx': `import {createElement} from 'react'; export default props => {globalThis.__factionPresentation.host = props;
      return createElement('div', null, props.main, props.open ? props.children : null);};`,
  }, (source, path) => path.endsWith('/race/FactionRaceView.tsx') ? source.replace("import { useCallback, useEffect, useMemo, useRef, useState } from 'react';",
    "import {useCallback, useEffect} from 'react'; const {useMemo, useRef, useState} = globalThis.__factionRenderHooks;") : undefined);
  try {
    const {default: View} = await vite.ssrLoadModule('/ui/src/charts/race/FactionRaceView.tsx');
    const { registerWidgets } = await vite.ssrLoadModule('/ui/src/dashboard/definitions.ts');
    const { definition } = await vite.ssrLoadModule('/ui/src/widgets/faction-strength/definition.ts');
    registerWidgets([definition]);
    const track = {id: 'track', axisOffset: 0, branch: {id: 'branch', firstHour: 0, lastHour: 100, gaps: []},
      source: {id: 'source', revision: '1', metadata: {campaignKey: 'campaign', countries: Array.from({length: 5000}, (_, index) => ({tag: String(index)}))}}};
    const props = {widget: {id: 'race', kind: 'faction-strength', title: 'Strength', trackIds: ['track'], countries: {kind: 'all'}, time: {mode: 'follow'}},
      runtime: {client: {}, tracks: [track], clock: {getHour: () => state.hour, getSnapshot: () => ({playing: true})},
        addWidgetBelow: (_id: string, pin: import('../../dashboard/model').WidgetSpec) => state.pins.push(pin)},
      options: {view: 'race'}, source: 'factories', max: Number.MAX_SAFE_INTEGER, label: 'Strength', measureLabel: () => 'Count', formatValue: String, active: true, onDiagnostics() {} };
    const render = () => {state.cursor = 0; return renderToStaticMarkup(createElement(View, props));};
    const html = render();
    assert.equal(state.requested, Number.MAX_SAFE_INTEGER, 'missing artwork does not impose a guessed inventory cap');
    assert.ok(state.chart, 'numeric chart still mounts');
    assert.equal(state.chart.entityLooks.color('faction-0'), '#888');
    assert.equal(state.chart.ranked.length, 5000);
    assert.doesNotMatch(html, /Synthetic colour failure|colours could not/);
    state.looks.error = undefined; state.looks.factions = Array.from({length: 5000}, (_, index) => ({id: String(index)}));
    state.looks.colours = new Map([['faction-0', '#123456']]); render();
    assert.equal(state.chart!.entityLooks.color('faction-0'), '#123456');
    state.looks.loading = true; state.looks.colours = new Map(); render();
    assert.equal(state.chart!.entityLooks.color('faction-0'), '#123456', 'pending colour refresh retains its known look');
    state.looks.loading = false; state.looks.error = new Error('Synthetic completed failure'); render();
    assert.equal(state.chart!.entityLooks.color('faction-0'), '#888', 'completed failures fall back to neutral');
    state.chart!.onPick(5, '1'); render(); assert.equal(state.detail!.hour, 10); assert.equal(state.detail!.following, true);
    state.host!.onPause(); render(); state.hour = 20; render(); assert.equal(state.detail!.hour, 10);
    state.host!.onPin(); const pin = JSON.parse(JSON.stringify(state.pins.at(-1)!));
    assert.deepEqual(pin.time, {mode: 'pin', windows: {track: {firstOffset: 10, lastOffset: 10}}});
    state.host!.onFollow(); render(); assert.equal(state.detail!.hour, 20);
    const stalePause = state.host!.onPause;
    props.runtime.client = {}; render(); state.chart!.onPick(5, '1'); render(); stalePause(); render();
    assert.equal(state.detail!.following, true, 'replaced clients cannot pause a new faction drill');
    props.widget = pin; props.options = pin.options; state.slots = []; render();
    assert.equal(state.detail!.hour, 10, 'a saved faction drill restores its paused date');
    assert.match(render(), /aria-pressed="false"/);
    state.control!.onFollow(); render(); assert.equal(state.detail!.hour, 20);
    state.control!.onPause(); render(); state.hour = 30; render(); assert.equal(state.detail!.hour, 20);
    state.control!.onFollow(); render(); assert.equal(state.detail!.hour, 30, 'restored faction pins can resume after pause');
  } finally { await vite.close(); delete host.__factionPresentation; delete host.__factionRenderHooks; }
});
