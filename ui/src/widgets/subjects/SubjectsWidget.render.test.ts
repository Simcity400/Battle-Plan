import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { TimelineProps } from '../../charts/timeline/Timeline';
import type { LegendItem } from '../../charts/Legend';
import type { CountryGroupReply, SubjectReply } from '../../generated/dashboard';

test('subject legends keep missing and folded level names without raw tokens or cadence notes', async () => {
  const levels = ['opaque-unnamed', 'mod-level-a', 'mod-level-b'].map((token, index) => ({ token, name: index ? `Mod level ${index}` : null, ordered: true }));
  const data: SubjectReply = { branchId: '1', levels, warnings: [], unobserved: [], runs: levels.map((level, index) => ({
    overlord: 'O', subject: `S${index}`, startHour: 0, firstHour: 0, lastHour: 10,
    steps: [{ firstHour: 0, lastHour: 10, level: level.token }],
  })) };
  const groups: CountryGroupReply = { branchId: '1', humans: [], factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [], warnings: [],
    unobserved: { humans: [], factions: [], wars: [] }, countryIdentities: ['O', 'S0', 'S1', 'S2'].map((tag, slot) => ({ tag, identities: [{ firstHour: 0, lastHour: 10, slot, countryId: slot + 1 }] })) };
  const state = { data, groups, legend: [] as LegendItem[], timeline: undefined as TimelineProps | undefined };
  const host = globalThis as typeof globalThis & { __subjectLegend?: typeof state }; host.__subjectLegend = state;
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': 'export const useProduct = (client, request, active, validate, source) => { if (source?.id !== request.sourceId || request.product !== "subjects") throw new Error("Subjects require prepared loading"); return { data: globalThis.__subjectLegend.data }; };',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__subjectLegend.groups });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => {};',
    '/dashboard/useCountriesAt.ts': 'export const useCountriesAt = () => ({ country: tag => ({ name: tag }), revision: 0 });',
    '/dashboard/useHoverLabels.ts': 'export const useHoverLabels = () => ({ ask() {}, labelAt: () => tag => ({ name: tag }) });',
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ categories: ["#123456"], text: "#ffffff", surface: "#000000" }); export const useViewportHeight = () => 1000;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ menu: null });',
    '/charts/Legend.tsx': 'export default props => { globalThis.__subjectLegend.legend = props.items; return null; };',
    '/charts/timeline/Timeline.tsx': 'export default props => { globalThis.__subjectLegend.timeline = props; return null; };',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'subject-level-presentation', enforce: 'pre', transform(_code, id) {
      return Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1];
    } }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/subjects/SubjectsWidget.tsx');
    const props = { widget: { id: 'subjects', kind: 'subjects', trackIds: ['main'], countries: { kind: 'all' }, time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: { id: '1', firstHour: 0, lastHour: 10 } }],
        client: {}, clock: { getHour: () => 10, getSnapshot: () => ({ hoursPerSecond: 0 }) } }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    renderToStaticMarkup(createElement(Widget, props));
    assert.deepEqual(state.legend.map(item => [item.label, item.tooltip]), [['Autonomy level name unavailable', undefined], ['Other levels', 'Mod level 1, Mod level 2']]);
    assert.ok(!JSON.stringify(state.legend.map(item => [item.label, item.tooltip])).includes('opaque-unnamed'));
    const chart = state.timeline!;
    for (const lane of chart.lanes) {
      const content = chart.tooltip!(lane, null, 5, true)!;
      assert.equal(content.note, undefined);
      if (lane.depth) assert.ok(content.rows.some(row => row.label === 'Subject since'));
    }
  } finally { delete host.__subjectLegend; await vite.close(); }
});
