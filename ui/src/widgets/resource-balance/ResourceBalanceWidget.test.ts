import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, isValidElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { ResourceBalanceReply } from '../../generated/dashboard';
import type { DrillState } from '../../charts/drill/drillState';
import type { ComponentProps, ReactElement, ReactNode } from 'react';
import type BreakdownChart from '../../charts/drill/BreakdownChart';
import { resourceCountryToken } from './resourceDrill.ts';

function followButton(node: ReactNode): (() => void) | undefined {
  if (Array.isArray(node)) return node.map(followButton).find(Boolean);
  if (!isValidElement<{ children?: ReactNode; onClick?: () => void }>(node)) return undefined;
  return node.type === 'button' && node.props.children === 'Follow playback' ? node.props.onClick : followButton(node.props.children);
}

test('the mounted resource pin dates its header and body to local picks and actual-clock Follow through a rename', async () => {
  const pool = { firstHour: 100, lastHour: 120, startsSegment: true, producedRaw: '100000', subjectsRaw: '0', importedRaw: '0',
    toExportRaw: '0', exportedRaw: '0', availableRaw: '100000', afterProjectsRaw: '100000', productionRaw: '0' };
  const data: ResourceBalanceReply = { branchId: 'b', tag: 'X', decimals: 5, warnings: [],
    resources: [{ id: 7, token: 'mod-resource', ordinal: 1, name: 'Synthetic resource', icon: null }],
    series: [{ id: 7, runs: [pool] }], trades: [], dealHours: [{ firstHour: 100, lastHour: 120 }] };
  const state = { data, hour: 112, drill: { breakdown: null, follow: true, hour: null } as DrillState,
    chart: undefined as ComponentProps<typeof BreakdownChart> | undefined,
    chip: undefined as ReactElement<{ hour: number; label: { name: string } | null }> | undefined,
    button: undefined as (() => void) | undefined, wanted: [] as { tag: string; hour: number }[] };
  const globals = globalThis as typeof globalThis & { __resourcePin?: typeof state };
  globals.__resourcePin = state;
  const replacements: Record<string, string> = {
    '/widgets/resource-balance/useResourceBalance.ts': `
      export const useResourceBalance = (_widget, runtime) => ({ track: runtime.tracks[0], tag: 'X',
        data: globalThis.__resourcePin.data, numbers: globalThis.__resourcePin.data });
      export const useResourceCountry = selection => ({ data: globalThis.__resourcePin.data, numbers: globalThis.__resourcePin.data,
        spans: selection ? new Map([['X', [{ firstHour: 100, lastHour: 109 }]], ['Y', [{ firstHour: 110, lastHour: 114 }]]]) : null, reason: null });`,
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => globalThis.__resourcePin.hour;',
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = (_client, _track, wanted) => {
      globalThis.__resourcePin.wanted = wanted;
      return { labelAt: hour => tag => ({ name: tag + ' at ' + hour, known: true, flag: 'data:flag-' + tag + '@' + hour }) };
    };`,
    '/dashboard/CountryComparisons.tsx': 'export default () => { throw new Error("An explicit native pin must keep one country."); };',
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({ categories: ['#123456'], muted: '#777777' });
      export const useViewportHeight = () => 1000;`,
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ menu: null, open() {} });',
    '/charts/drill/BreakdownChart.tsx': 'export default props => { globalThis.__resourcePin.chart = props; return String(props.selectedHour); };',
    '/charts/drill/DrillParts.tsx': 'export default () => null;',
    '/charts/useRowTooltips.ts': 'export const useRowTooltips = () => ({ hovered: null, descriptionId: String, rowProps: () => ({ onKeyDown() {} }) });',
    '/widgets/resource-balance/ResourceDealsView.tsx': 'export default () => null;',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'resource-pin-dates', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        const replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        if (path.endsWith('/widgets/resource-balance/ResourceBalanceWidget.tsx')) return source
          .replace(/\buseContext\(/g, '(() => null)(')
          .replace(/\buseMemo(?:<[^;\n]*>)?\(/g, '((factory) => factory())(')
          .replace(/\buseCallback(?:<[^;\n]*>)?\(/g, '((callback) => callback)(')
          .replace(/\buseEffect\(/g, '(() => {})(')
          .replace(/\buseReducer\(/g, '((reduce) => [globalThis.__resourcePin.drill, action => { globalThis.__resourcePin.drill = reduce(globalThis.__resourcePin.drill, action); }])(')
          .replace(/\buseState(?:<[^;\n]*>)?\(/g, '((initial) => [initial, () => {}])(')
          .replace(/\buseRef(?:<[^;\n]*>)?\(/g, '((initial) => ({ current: initial }))(')
          .replace(/\bcreatePortal\(/g, '((chip) => { globalThis.__resourcePin.chip = chip; return null; })(')
          + '\nexport { ResourceBalanceCountry, BalanceBars };';
      },
    }] });
  try {
    const { default: Widget, ResourceBalanceCountry: Country, BalanceBars: Bars } = await vite.ssrLoadModule('/ui/src/widgets/resource-balance/ResourceBalanceWidget.tsx');
    const props = { widget: { id: 'resources', kind: 'resource-balance', title: 'Resources', trackIds: ['track'],
      countries: { kind: 'tags', tags: ['X'] }, time: { mode: 'pin', windows: { track: { firstOffset: 5, lastOffset: 5 } } },
      options: { show: 'resource', resourceCountry: resourceCountryToken({ campaign: 'c', branch: 'b', tag: 'X', originOffset: 5 }) } },
      runtime: { client: {}, tracks: [{ id: 'track', axisOffset: 0, branch: { id: 'b', firstHour: 100, lastHour: 120 },
        source: { id: 'source', revision: '1', metadata: { campaignKey: 'c' } } }],
        clock: { getHour: () => state.hour, getSnapshot: () => ({ playing: false, hoursPerSecond: 0 }) } },
      countrySlot: {} as HTMLElement, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    assert.equal(Widget(props).type, Country);
    const render = () => {
      const node = Country(props);
      state.button = followButton(node);
      return renderToStaticMarkup(node);
    };
    assert.match(render(), /Follow playback/);
    assert.equal(state.chart!.selectedHour, 105);
    assert.equal(state.chip!.props.label!.name, 'X at 105');
    state.chart!.onPick!(113);
    render();
    assert.equal(state.chip!.props.hour, 113);
    assert.equal(state.chip!.props.label!.name, 'Y at 113');
    state.button!();
    render();
    assert.equal(state.chart!.selectedHour, 112);
    assert.equal(state.chip!.props.label!.name, 'Y at 112');
    state.hour = 119;
    render();
    assert.equal(state.chip!.props.label, null, 'the tag replacement never supplies a header');
    assert.equal(state.chip!.props.hour, 119);
    assert.deepEqual(props.widget.time, { mode: 'pin', windows: { track: { firstOffset: 5, lastOffset: 5 } } });

    const { resourceCountry: _removed, ...legacyOptions } = props.widget.options!;
    props.widget.options = legacyOptions;
    state.hour = 112;
    state.drill = { breakdown: null, follow: true, hour: null };
    render();
    assert.equal(state.chip!.props.label!.name, 'X at 105', 'a legacy fixed-date pin retains the actor at its saved date');
    state.button!();
    render();
    assert.equal(state.chip!.props.label!.name, 'Y at 112');
    props.widget.time = { mode: 'follow' };
    assert.match(render(), /Saved country selection unavailable/);
    assert.equal(state.chip!.props.label, null);

    const signed = { ...data, series: [{ id: 7, runs: [{ ...pool, producedRaw: '-100000', importedRaw: '-200000',
      toExportRaw: '-100000', availableRaw: '-200000', afterProjectsRaw: '-200000' }] }] };
    const bars = (reply: ResourceBalanceReply) => renderToStaticMarkup(createElement(Bars, { data: reply, tokens: { categories: ['#123', '#456'] },
      hour: 105, order: 'game', scale: 'moment', hideEmpty: true, owner: 'signed', breakdown: () => null, onPartners() {},
      active: true, hoursPerSecond: 0, onMenu() {}, onPick() {} }));
    assert.match(bars(signed), /role="button"[^>]*aria-label="Synthetic resource: supply/,
      'Hide empty preserves negative supply and demand');
    assert.doesNotMatch(bars({ ...data, series: [{ id: 7, runs: [{ ...pool, producedRaw: '0', availableRaw: '0', afterProjectsRaw: '0' }] }] }), /role="button"/);
  } finally { await vite.close(); delete globals.__resourcePin; }
});
