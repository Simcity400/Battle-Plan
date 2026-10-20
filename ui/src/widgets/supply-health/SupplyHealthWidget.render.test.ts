import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import type { CountryGroupReply, SupplyHealthReply } from '../../generated/dashboard';

test('supply of a group member over hours still being prepared is loading, not unrecorded', async () => {
  const reply: SupplyHealthReply = { branchId: BRANCH.id, available: true, reason: null, missingInputs: [], warnings: [],
    series: [{ tag: 'GER', runs: [{ firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour, startsSegment: true, noSupply: 1, strained: 2, inSupply: 3 }] }] };
  const state = { groups: pendingGroups() as CountryGroupReply };
  const host = globalThis as typeof globalThis & { __supplyPending?: typeof state }; host.__supplyPending = state;
  // Rows mount once measured; the fixture gives them their height up front.
  const widget = readFileSync(new URL('./SupplyHealthWidget.tsx', import.meta.url), 'utf8')
    .replace('const [bodyHeight, setBodyHeight] = useState(0);', 'const [bodyHeight, setBodyHeight] = useState(100);');
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/supply-health/SupplyHealthWidget.tsx', {
    '/widgets/supply-health/SupplyHealthWidget.tsx': widget,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__supplyPending.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 0; export const usePlaybackSpeed = () => 0;',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ revision: 0, labelAt: () => tag => ({ name: tag }) });',
    '/charts/useChartTokens.ts': "export const useChartTokens = () => ({ categories: ['#123'], muted: '#888', surface: '#000' }); export const useViewportHeight = () => 1000;",
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/widgets/supply-health/preparedSupply.ts': `export const usePreparedSupply = () => ({ data: ${JSON.stringify(reply)}, geometry: new Map(), summaries: new Map(), retry() {} });`,
    '/charts/WindowedRows.tsx': "import { createElement, Fragment } from 'react'; export default props => createElement(Fragment, {}, props.items.map(item => createElement(Fragment, { key: item.key }, props.children({ item: item.item, phase: 'entered' }))));",
    '/charts/StackedShareChart.tsx': 'export default () => "supply chart";',
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
  });
  try {
    // A range wholly after the hours the groups reply has read.
    const props = { widget: { id: 'supply', kind: 'supply-health', trackIds: ['main'], countries: humansBy('hour'),
      time: { mode: 'range', windows: { main: { firstOffset: 170, lastOffset: BRANCH.lastHour } } }, options: {} },
    runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
      clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ hoursPerSecond: 0, playing: false }) }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // GER's membership after hour 160 is still being prepared.
    const pending = render();
    assert.match(pending, /aria-label="This range is still being prepared\."/);
    assert.doesNotMatch(pending, /Supply was not recorded/);
    // The final reply proves GER was no human player then, so nothing is drawn for it.
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /Supply was not recorded for this period\./);
  } finally { delete host.__supplyPending; await close(); }
});
