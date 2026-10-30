import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { CountryGroupReply, SubjectReply, AutonomyReply, TensionReply } from '../../generated/dashboard';
import { tokensFromValues } from '../tokens.ts';

test('native world callers bind held names/flags to holder boundaries without rename events', async () => {
  const groups: CountryGroupReply = { branchId: '1', humans: [], wars: [], warStarts: [], capitulations: [], warnings: [],
    countryIdentities: [
      { tag: 'A', identities: [{ firstHour: 0, lastHour: 9, slot: 1, countryId: 10 },
        { firstHour: 10, lastHour: 16, slot: 2, countryId: 20 }, { firstHour: 18, lastHour: 20, slot: 2, countryId: 20 }] },
      { tag: 'B', identities: [{ firstHour: 10, lastHour: 20, slot: 1, countryId: 10 }] },
      { tag: 'C', identities: [{ firstHour: 0, lastHour: 9, slot: 2, countryId: 20 }] },
      { tag: 'OVR', identities: [{ firstHour: 0, lastHour: 20, slot: 3, countryId: 30 }] },
    ],
    identityChanges: [],
    unobserved: { humans: [], factions: [], wars: [] }, factions: [{ id: '1', firstHour: 0, lastHour: 20,
      name: 'Synthetic faction', recordedName: 'Synthetic faction', nameReason: null,
      names: [{ firstHour: 0, lastHour: 20, name: 'Synthetic faction', reason: null }], incomplete: [],
      members: [{ tag: 'A', firstHour: 0, lastHour: 20 }, { tag: 'B', firstHour: 10, lastHour: 20 }, { tag: 'C', firstHour: 0, lastHour: 9 }],
      leaders: [{ tag: 'A', firstHour: 0, lastHour: 9 }, { tag: 'B', firstHour: 10, lastHour: 20 }] }] };
  const subjects: SubjectReply = { branchId: '1', levels: [], unobserved: [], warnings: [], runs: [{
    overlord: 'OVR', subject: 'A', startHour: 0, firstHour: 0, lastHour: 20, steps: [{ firstHour: 0, lastHour: 20, level: null }] }] };
  const autonomy: AutonomyReply = { branchId: '1', scoreScale: 100000, levels: [{ id: 'synthetic', name: 'Synthetic level', defined: true }],
    warnings: [], recorded: [{ firstHour: 0, lastHour: 20 }], unidentified: [], subjects: [{ tag: 'A', runs: [{
      firstHour: 0, lastHour: 20, startsSegment: true, overlord: 'OVR', level: 0, scoreRaw: '100', lowerRaw: '0', upperRaw: '200',
      ratioRaw: '0', lowest: true, highest: true }] }] };
  const tension: TensionReply = { branchId: '1', tensionScale: 1000, contributionScale: 100000, ceilingRaw: '10000000', warnings: [], events: [],
    reads: [{ firstHour: 0, lastHour: 20, startsSegment: true, tensionRaw: '1000', sumRaw: '100000', unnamedRaw: '0' }],
    countries: [{ tag: 'A', slot: 2, runs: [{ firstHour: 10, lastHour: 20, startsSegment: true, valueRaw: '100000' }], identities: groups.countryIdentities[0].identities.filter(identity => identity.slot === 2) }] };
  const state = { groups: groups as CountryGroupReply | undefined, products: { subjects, autonomy, 'world-tension': tension },
    segment: undefined as ((tag: string, hour: number) => number | null) | undefined };
  const host = globalThis as typeof globalThis & { __timelineActors?: typeof state };
  host.__timelineActors = state;
  const tokens = tokensFromValues(() => '#123456', 'sans-serif');
  const replacements: Record<string, string> = {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__timelineActors.groups, retry() {} });',
    '/dashboard/products.ts': 'export const useProduct = (_client, request) => ({ data: request === null ? undefined : globalThis.__timelineActors.products[request.product], retry() {} }); export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/dashboard/useCountriesAt.ts': `export const useCountriesAt = (_client, _track, _tags, _hour, _active, _names, segment) => {
      globalThis.__timelineActors.segment = segment; return { country: tag => ({ name: tag }), revision: 0 }; };`,
    '/dashboard/countryLooks.ts': `export const useCountryLooks = (_client, _track, _tags, _hour, _active, segment) => {
      globalThis.__timelineActors.segment = segment; return { name: tag => tag, flag: () => undefined, flagPending: () => false,
        flagReason: () => undefined, color: () => null, revision: 0 }; };`,
    '/dashboard/useHoverLabels.ts': 'export const useHoverLabels = (_client, _track, _active, country) => ({ ask() {}, labelAt: () => country, revision: 0 });',
    '/dashboard/FactionColours.tsx': `export const useFactionColours = () => ({ colours: new Map([['1', '#123456']]),
      names: faction => ({ name: faction.name, reason: null }), warnings: [], setColour() {} });`,
    '/charts/useChartTokens.ts': `export const useChartTokens = () => (${JSON.stringify(tokens)}); export const useViewportHeight = () => 500;`,
    '/charts/timeline/Timeline.tsx': 'export default () => null;',
    '/charts/TimeChart.tsx': 'export default () => null;',
    '/charts/drill/DrillHost.tsx': 'export default () => null;',
    '/charts/Legend.tsx': 'export default () => null;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ menu: null, open() {} });',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'timeline-native-actor-boundary', enforce: 'pre',
      transform(_source, id) { return Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1]; } }] });
  try {
    for (const [kind, file] of [['factions', 'FactionsWidget'], ['subjects', 'SubjectsWidget'], ['autonomy', 'AutonomyWidget'], ['world-tension', 'WorldTensionWidget']]) {
      const { default: Widget } = await vite.ssrLoadModule(`/ui/src/widgets/${kind}/${file}.tsx`);
      const track = { id: 'track', axisOffset: 0, branch: { id: '1', firstHour: 0, lastHour: 20, gaps: [] },
        source: { id: 'source', revision: '1', metadata: { countries: [], branches: [], playerCountryId: null } } };
      const props = { widget: { id: kind, kind, trackIds: ['track'], countries: { kind: 'all' }, time: { mode: 'campaign' }, options: {} },
        runtime: { tracks: [track], client: {}, clock: { getHour: () => 15, getSnapshot: () => ({ playing: false, hoursPerSecond: 0 }) } },
        active: true, onDiagnostics() {} } as unknown as WidgetProps;
      state.groups = groups; state.segment = undefined;
      renderToStaticMarkup(createElement(Widget, props));
      assert.equal(typeof state.segment, 'function', `${kind} must supply its actor boundary to the looks hook`);
      assert.equal(state.segment!('A', 9), 0, `${kind}: native looks begin at their recorded holder run`);
      assert.equal(state.segment!('A', 10), 10, `${kind}: a reused A must never hold its old actor's flag`);
      assert.equal(state.segment!('B', 15), 10, `${kind}: a renamed tag starts at the native boundary`);
      assert.equal(state.segment!('A', 17), null, `${kind} must not carry its native holder across an identity gap`);
      assert.equal(state.segment!('A', 18), 18, `${kind} resumes held looks only at the next recorded identity run`);
      assert.equal(state.segment!('missing', 15), null, `${kind}: missing metadata certifies no holder`);
      state.groups = undefined; state.segment = undefined;
      renderToStaticMarkup(createElement(Widget, props));
      assert.equal(state.segment!('A', 15), null, `${kind}: pending aliases cannot certify an empty default Lineage`);
    }
  } finally { delete host.__timelineActors; await vite.close(); }
});
