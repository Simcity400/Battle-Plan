import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { DashboardSource } from './client';
import { registerWidgets } from './definitions.ts';
import type { WidgetProps } from './definitions';
import { bindTracks, clockBounds, createWidget } from './model.ts';
import { PlaybackController } from './playback.ts';
import type { ConstructionLogManifestReply, ConstructionProject, ConstructionSpendingReply, CountryGroupReply } from '../generated/dashboard';
import { definition as logDefinition } from '../widgets/construction-log/definition.ts';
import { logRows, mergeWindows } from '../widgets/construction-log/constructionData.ts';
import { definition as spendingDefinition } from '../widgets/construction-spending/definition.ts';
import { placeName, stateNameRequests } from './construction.ts';
import type { StateNameAt } from './construction';

test('construction batches every state at one shown hour, including 5000 mod states and route endpoints', () => {
  const places = Array.from({ length: 5000 }, (_, index) => ({ state: index + 1, toState: index + 1 }));
  places.push({ state: 1, toState: 5000 });
  const wanted = stateNameRequests(places, 124);
  assert.equal(wanted.length, 5000);
  assert.ok(wanted.every(request => request.hour === 124));
  assert.deepEqual([wanted[0].state, wanted.at(-1)?.state], [1, 5000]);
  assert.deepEqual(stateNameRequests(places, null), []);
});

test('construction preserves shared legacy fallback, capture failure and language refusal without another name lookup', () => {
  const calls: [number, number][] = [];
  const names: StateNameAt = (hour, state) => {
    calls.push([hour, state]);
    return { state, name: hour === 100 ? 'Authored fallback for old prefix' : hour === 124 ? 'Recorded rename' : null,
      missing: hour === 130 ? 'Stored name capture failed' : hour === 148 ? 'Stored name belongs to another recorded language' : null };
  };
  const place = { state: 9000, toState: 9000, railway: true };
  assert.equal(placeName(place, 100, names).label, 'Authored fallback for old prefix');
  assert.equal(placeName(place, 124, names).label, 'Recorded rename');
  for (const hour of [130, 148]) {
    const unresolved = placeName(place, hour, names);
    assert.equal(unresolved.label, 'State 9000');
    assert.match(unresolved.missing ?? '', hour === 130 ? /capture failed/ : /recorded language/);
  }
  assert.deepEqual(calls, [[100, 9000], [124, 9000], [130, 9000], [148, 9000]]);
});

test('distinct railway state IDs retain both endpoints even when their recorded names match', () => {
  const calls: number[] = [];
  const names: StateNameAt = (_hour, state) => { calls.push(state); return { state, name: 'Metro', missing: null }; };
  assert.equal(placeName({ state: 5, toState: 6, railway: true }, 124, names).label, 'Metro to Metro');
  assert.deepEqual(calls, [5, 6]);
  calls.length = 0;
  assert.equal(placeName({ state: 5, toState: 5, railway: true }, 124, names).label, 'Metro');
  assert.deepEqual(calls, [5]);
});

test('both construction renderers release dated-name demand outside their branch while preserving held dates', async () => {
  registerWidgets([logDefinition, spendingDefinition]);
  const source = (id: string, firstHour: number, lastHour: number): DashboardSource => ({ id, path: `C:/recordings/${id}.battleplan`, revision: '1',
    metadata: { path: `C:/recordings/${id}.battleplan`, revision: '1', name: id, format: 1, captureMode: 'full', campaignKey: 'c0ffee',
      branches: [{ id: '1', sessionId: '1', firstHour, lastHour, parentId: null, parentHour: null, sampleBoundary: 'Sample origin unknown', gaps: [] }],
      countries: [], playerCountryId: null, contentVerified: false, partial: null, warnings: [] } });
  const tracks = bindTracks({ version: 1, name: 'Comparison', alignment: 'calendar', widgets: [], layouts: {},
    roles: ['short', 'long'].map(id => ({ id, label: id, selectedTag: null })),
    tracks: ['short', 'long'].map(id => ({ id, roleId: id, label: id, branchIndex: 0, anchorOffset: null })) },
  new Map([['short', source('short', 100, 200)], ['long', source('long', 0, 1000)]]));
  const clock = new PlaybackController();
  clock.setBounds(clockBounds(tracks), true);
  const project: ConstructionProject = { id: '1', kind: 'building', tag: 'synthetic-country', building: 0, repair: false, state: 7,
    toState: null, province: null, routeProvinces: null, baseLevel: null, targetLevel: null, costRaw: '100', createdHour: 90, firstRead: 100,
    runs: [{ firstHour: 100, lastHour: 200, factories: 1, remaining: 1, repairLevels: 0 }],
    earlierUnknown: false, built: [], unaccounted: [], possible: [], added: [], end: null, firstRepairTick: null };
  const manifest: ConstructionLogManifestReply = { costScale: 100000, windows: [{ firstHour: 100, lastHour: 200 }], recorded: true,
    repairIcon: null, types: [{ id: 777, token: 'synthetic_mod_building', name: 'Building', icon: null }], warnings: [] };
  const log = mergeWindows(manifest, new Map([[0, { window: 0, interval: manifest.windows[0], outputReads: [], outputUnavailable: [],
    projects: [project], outputs: [], warnings: [] }]]), 0);
  const spending: ConstructionSpendingReply = { branchId: '1', tag: project.tag, costScale: manifest.costScale, reads: [100, 124, 200],
    railwayReads: [], repairIcon: null, types: manifest.types, states: [], beforeFirstRead: 0, unattributed: [], warnings: [],
    leaves: [{ repair: false, kind: 'building', building: 0, state: 7, toState: null, spent: [{ knownAt: 100, totalRaw: '10' }], unknown: [],
      projects: [{ project: '1', firstRead: 100, knownAt: null, until: null, unread: false, resumes: null }] }] };
  type Demand = { kind: 'country' | 'state'; active: boolean; wanted: readonly { hour: number; tag?: string; state?: number }[] };
  const groups: CountryGroupReply = { branchId: '1', humans: [], factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [],
    countryIdentities: [{ tag: project.tag, identities: [{ firstHour: 100, lastHour: 200, slot: 1, countryId: 1 }] }],
    unobserved: { humans: [], factions: [], wars: [] }, warnings: [] };
  const fixture = { track: tracks[0], log, spending, groups, demands: [] as Demand[],
    visible: logRows(log, 124, { types: [], projects: 'all', country: () => true, from: null }, 'change') };
  const host = globalThis as typeof globalThis & { __constructionDemand?: typeof fixture };
  host.__constructionDemand = fixture;
  // Keep the real renderers/model; retain a reached viewport row and observe their shared-hook request boundary.
  const replacements: Record<string, string> = {
    '/construction-log/useConstructionLog.ts': `export const useConstructionLog = (_runtime, _track, _active, hour) =>
      ({manifest: globalThis.__constructionDemand.log.manifest, log: globalThis.__constructionDemand.log, hour, onDrawn() {}});`,
    '/dashboard/products.ts': 'export const useProduct = () => ({data: globalThis.__constructionDemand.spending}); export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({data: globalThis.__constructionDemand.groups});',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => undefined;',
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = (_client, _track, wanted, active) => {
      globalThis.__constructionDemand.demands.push({kind: 'country', wanted, active});
      return {labelAt: () => tag => ({name: tag})};
    };`,
    '/charts/log/LogList.tsx': 'export default () => null;',
    '/charts/sunburst/Sunburst.tsx': 'export default () => null;',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'construction-demand-fixture', enforce: 'pre',
      resolveId(id) { if (id.endsWith('/dashboard/useStateNamesAt')) return '\0construction-state-names'; },
      load(id) { if (id === '\0construction-state-names') return `export const useStateNamesAt = (_client, _track, wanted, active) => {
        globalThis.__constructionDemand.demands.push({kind: 'state', wanted, active});
        return (_hour, state) => ({state, name: 'Recorded state', missing: null});
      };`; },
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/construction-log/ConstructionLogWidget.tsx')) {
          return source.replace('useMemo, useState', 'useMemo') + '\nconst useState = () => [globalThis.__constructionDemand.visible, () => {}];';
        }
        return Object.entries(replacements).find(([suffix]) => path.endsWith(suffix))?.[1];
      } }] });
  try {
    for (const [kind, path] of [['construction-log', 'construction-log/ConstructionLogWidget'], ['construction-spending', 'construction-spending/ConstructionSpendingWidget']]) {
      const { default: Widget } = await vite.ssrLoadModule(`/ui/src/widgets/${path}.tsx`);
      const widget = createWidget(kind, 'short', kind);
      widget.countries = { kind: 'tags', tags: [project.tag] };
      const render = (hour: number, active = true) => {
        clock.seek(hour);
        fixture.demands = [];
        return renderToStaticMarkup(createElement(Widget, { widget, runtime: { tracks, clock, client: {} }, active,
          onDiagnostics() {} } as unknown as WidgetProps));
      };
      const expectDemand = (hour: number | null) => {
        assert.deepEqual(fixture.demands.find(demand => demand.kind === 'state'), { kind: 'state', active: hour != null,
          wanted: hour == null ? [] : [{ state: 7, hour }] }, kind);
        if (kind === 'construction-log') assert.deepEqual(fixture.demands.find(demand => demand.kind === 'country'),
          { kind: 'country', active: hour != null, wanted: hour == null ? [] : [{ hour: 100, tag: project.tag }] }, 'country labels retain the native first-read date');
      };
      widget.time = { mode: 'follow' };
      for (const hour of [99, 400]) {
        assert.match(render(hour), /This time is outside the recording\./);
        expectDemand(null);
      }
      for (const hour of [100, 200]) { render(hour); expectDemand(hour); }
      widget.time = { mode: 'pin', windows: { short: { firstOffset: 0, lastOffset: 50 } } };
      render(400); expectDemand(150);
      widget.time = { mode: 'pin', windows: { short: { firstOffset: 0, lastOffset: 300 } } };
      assert.match(render(400), /This time is outside the recording\./); expectDemand(null);
      widget.time = { mode: 'campaign' };
      render(400); expectDemand(200);
      widget.time = { mode: 'range', firstOffset: -50, lastOffset: 30 };
      render(400); expectDemand(130);
      widget.time = { mode: 'follow' };
      render(124, false); expectDemand(null);
    }
  } finally { delete host.__constructionDemand; await vite.close(); }
});
