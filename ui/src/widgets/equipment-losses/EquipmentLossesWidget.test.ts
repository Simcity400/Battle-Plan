import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { DashboardSource } from '../../dashboard/client';
import { registerWidgets } from '../../dashboard/definitions.ts';
import { bindTracks, createWidget } from '../../dashboard/model.ts';
import { PlaybackController } from '../../dashboard/playback.ts';
import type { LossesReply } from '../../generated/dashboard';
import { definition } from './definition.ts';

registerWidgets([definition]);

const losses: LossesReply = {
  branchId: '1', ships: [], from: 100, to: 110, width: 64, exactHours: [], restarts: [], tag: 'GER', amountScale: 100_000, landSource: 'combatLog', landLowerBound: null,
  destructionRuns: [{ firstHour: 100, lastHour: 109, countedFrom: 100, withheldFrom: null }], destructionCoverage: [{ firstHour: 100, lastHour: 109, reason: null }], retainedMonths: 7, warnings: [],
  designs: [{ variant: 91001, equipment: 'mod_walkers_v2', archetype: 'mod_walkers', archetypeName: 'Walking machines', names: [{ firstHour: 100, name: 'Walker', known: true }] }],
  coverage: [{ firstHour: 100, lastHour: 109, coverage: 'logged' }],
  causes: [{ source: { kind: 'combatLog', reason: 0 }, domain: 'land', name: { key: 'mod_loss_combat', template: 'Mod combat', text: 'Mod combat' }, group: null, loss: true,
    unknown: null, unknownSpans: [], leaves: [{ design: 0, key: '0', points: [{ hour: 100, totalRaw: '200000' }, { hour: 105, totalRaw: '350001' }], exact: [], lowerBoundFrom: null }] }],
};
const source: DashboardSource = { id: 'short', path: 'C:/recordings/short.battleplan', revision: '1', metadata: { path: 'C:/recordings/short.battleplan', revision: '1',
  name: 'short', format: 1, captureMode: 'full', campaignKey: 'c0ffee', branches: [{ id: '1', sessionId: '1', firstHour: 100, lastHour: 109,
    main: true, fork: null, joins: [], sampleBoundary: 'Sample origin unknown', gaps: [] }],
  countries: [], playerCountryId: null, contentVerified: false, partial: null, warnings: [], buildWarnings: [] } };
const host = globalThis as typeof globalThis & { __losses?: LossesReply; __breakdownHost?: { bounds: readonly [number, number] | null; pin: number | null } | null };

test('Equipment losses binds the breakdown host with branch bounds so hours past the track end resolve outside the recording', async () => {
  const tracks = bindTracks({ version: 1, name: 'Short', alignment: 'calendar', widgets: [], layouts: {}, roles: [{ id: 'short', label: 'short', selectedTag: 'GER' }],
    tracks: [{ id: 'short', roleId: 'short', label: 'short', branchIndex: 0, anchorOffset: null }] }, new Map([['short', source]]));
  const clock = new PlaybackController();
  clock.setBounds({ firstHour: 100, lastHour: 200 }, true);
  host.__losses = losses;
  const replacements: Record<string, string> = {
    '/widgets/land-battles/CombatLogDetails.tsx': 'export default () => null;',
    '/widgets/equipment-losses/lossesSource.ts': `export const stretchWindow = (_reply, window) => ({ ...window, since: null }); export const exactLossHour = () => true;
      export const useLossHistory = () => ({ data: globalThis.__losses, retry() {}, onSummaryView() {}, onHover() {}, navigate() {}, exactAt: async () => undefined });`,
    '/dashboard/headProduct.ts': `const live = { read: () => ({ get: () => undefined, missing: () => 'Not recorded' }), subscribe: () => () => {} };
      export const useHeadProduct = () => ({ outside: false, data: undefined, root() {}, live, at: null, atNow: () => null, shownAt: () => null, current: () => undefined, read: 'off', hour: null, retry() {} });`,
    '/dashboard/useCountryGroups.ts': `const data = { countryIdentities: [{ tag: 'GER', identities: [{ slot: 1, countryId: 1, firstHour: 100, lastHour: 109 }] }] };
      export const useCountryGroups = () => ({ data, retry() {} });`,
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = () => ({ labelAt: () => () => ({ name: 'Germany' }) });`,
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({ categories: ['#808080'] }); export const useViewportHeight = () => 800;`,
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
    '/families/breakdown/FamilyBreakdownHost.tsx': 'export default props => { globalThis.__breakdownHost = { bounds: props.bounds, pin: props.pin }; return null; };',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'equipment-losses-fixture', enforce: 'pre',
      transform(_source, id) { return Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1]; } }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/equipment-losses/EquipmentLossesWidget.tsx');
    const widget = createWidget('equipment-losses', 'short', 'equipment-losses');
    widget.time = { mode: 'follow' };
    widget.countries = { kind: 'tags', tags: ['GER'] };
    const runtime = { tracks, clock, client: {}, addWidgetBelow() {} };
    clock.seek(109);
    host.__breakdownHost = null;
    renderToStaticMarkup(createElement(Widget, { widget, runtime, active: true, countrySlot: null, onDiagnostics() {} }));
    assert.deepEqual(host.__breakdownHost, { bounds: [100, 109], pin: null });
  } finally {
    await vite.close();
  }
});
