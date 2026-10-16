import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { plotFrame } from '../../charts/frame.ts';
import { keyedStop, keyStopList } from '../../charts/hover.ts';
import type { TimeChartProps } from '../../charts/TimeChart';
import type { ChartTokens } from '../../charts/tokens';
import type { WidgetProps } from '../../dashboard/definitions';
import type { CountryGroupReply, MeasureReply, PoliticsLedgerReply } from '../../generated/dashboard';

for (const measure of ['politicalPower', 'commandPower'] as const) test(`${measure} admits payment keyboard stops with their confirming read`, async () => {
  const tag = 'synthetic country', currency = measure === 'politicalPower' ? 'Political' : 'Command';
  const balance: MeasureReply = { branchId: 'branch', measure, decimals: 5, warnings: [], series: [{ tag, drops: [],
    runs: [{ firstHour: 0, lastHour: 30, valueRaw: '1000000', startsSegment: true }] }] };
  const ledger: PoliticsLedgerReply[] = [{ branchId: 'branch', tag, decimals: 5, warnings: [], daily: [], balanceChanges: [],
    spending: [{ id: 'synthetic payment', sequence: '1', ordinal: 0, hour: 24, readHour: 26, countryTag: tag, countryName: tag,
      currency, category: 'synthetic charges', itemId: 'synthetic action', chargeRaw: '100000', realizedDeltaRaw: '0', covered: true, occurrences: 1,
      changes: [{ readHour: 26, chargeRaw: '100000', realizedDeltaRaw: '0', covered: true, occurrences: 1 }] }],
    spendItems: [{ id: 'synthetic action', groupId: 'synthetic identity', name: 'Synthetic action', action: 'Recorded synthetic action', icon: null,
      tooltipToken: null, character: null, targetTag: null, targetName: null, unknown: null }],
    spendCategories: [{ id: 'synthetic charges', name: 'Synthetic charges', identified: true }],
    spendingCoverage: [{ readHour: 0, sinceHour: 0, throughHour: 30, political: true, command: true, reason: null }], commandHolds: [], ideaTooltips: null }];
  const groups: CountryGroupReply = { branchId: 'branch', humans: [], wars: [], warStarts: [], capitulations: [], identityChanges: [], warnings: [],
    countryIdentities: [{ tag, identities: [{ firstHour: 0, lastHour: 30, slot: 1, countryId: 1 }] }], factions: [],
    unobserved: { humans: [], factions: [], wars: [] } };
  type Slot = { value?: unknown; deps?: readonly unknown[] };
  const state = { balance, ledger, groups, clock: 25, cursor: 0, slots: [] as Slot[], effects: [] as (() => unknown)[],
    tokens: { surface: '#000000', muted: '#888888', categories: ['#123456'], countryColours: 'raw' } as ChartTokens,
    styles: { known: new Map([[tag, { tag, name: tag, color: [20, 50, 80] }]]), data: { warnings: [] } },
    chart: undefined as TimeChartProps | undefined,
    drill: undefined as { hour: number; mode: string } | undefined,
    drillHost: undefined as { onClose: () => void } | undefined };
  const host = globalThis as typeof globalThis & { __paymentFixture?: typeof state; __paymentHook?: (name: string, ...args: unknown[]) => unknown };
  host.__paymentFixture = state;
  host.__paymentHook = (name, ...args) => {
    const slot = state.slots[state.cursor++] ??= {};
    if (name === 'useMemo' || name === 'useCallback' || name === 'useEffect') {
      const [factory, deps] = args as [() => unknown, readonly unknown[]];
      if (!slot.deps || slot.deps.length !== deps.length || !deps.every((value, index) => Object.is(value, slot.deps![index]))) {
        slot.deps = [...deps];
        if (name === 'useMemo') slot.value = factory();
        else if (name === 'useCallback') slot.value = factory;
        else state.effects.push(factory);
      }
      return slot.value;
    }
    if (!('value' in slot)) slot.value = name === 'useReducer' ? args[1] : name === 'useRef' ? { current: args[0] }
      : typeof args[0] === 'function' ? (args[0] as () => unknown)() : args[0];
    if (name === 'useRef') return slot.value;
    return [slot.value, (next: unknown) => { slot.value = name === 'useReducer'
      ? (args[0] as (before: unknown, action: unknown) => unknown)(slot.value, next)
      : typeof next === 'function' ? (next as (before: unknown) => unknown)(slot.value) : next; }];
  };
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': `export const useProduct = (_client, request, active, validate) => ({
      data: request?.product === 'country-measure' && active ? validate(globalThis.__paymentFixture.balance) : undefined, retry() {} });
      export const useProducts = () => ({ data: globalThis.__paymentFixture.ledger, retry() {} });`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__paymentFixture.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => globalThis.__paymentFixture.clock;',
    '/dashboard/useCountryLineStyles.ts': 'export const useCountryLineStyles = () => globalThis.__paymentFixture.styles;',
    '/dashboard/countryNamesAt.ts': `export const nameKey = (hour, tag) => JSON.stringify([hour, tag]); export const useCountryNamesRead = (_client, _track, wanted) =>
      new Map(wanted.map(row => [nameKey(row.hour, row.tag), row.tag]));`,
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }), revision: 0 });',
    '/dashboard/countryLooks.ts': 'export const useCountryLooks = () => ({ flag() {}, flagPending: () => false, flagReason() {} });',
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => globalThis.__paymentFixture.tokens; export const useViewportHeight = () => 1000;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/charts/Legend.tsx': 'export default () => null;',
    '/charts/TimeChart.tsx': 'export default props => { globalThis.__paymentFixture.chart = props; return null; };',
    '/charts/drill/DrillHost.tsx': `import { createElement } from 'react'; export default props => { globalThis.__paymentFixture.drillHost = props;
      return createElement('div', null, props.main, props.open ? props.children : null); };`,
    '/widgets/country-measure/PowerDrill.tsx': 'export default props => { globalThis.__paymentFixture.drill = props; return null; };',
  };
  const vite = await createServer({ root: fileURLToPath(new URL('../../../../', import.meta.url)), configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'payment-admission-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/'), replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        if (path.endsWith('/widgets/country-measure/MeasureWidget.tsx')) return source.replace(/\b(useMemo|useCallback|useState|useReducer|useRef|useEffect)(?:<[\s\S]*?>)?\(/g,
          (_match, name) => `globalThis.__paymentHook(${JSON.stringify(name)}, `);
      },
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/country-measure/MeasureWidget.tsx');
    const props = { config: { measure, title: currency + ' power', noun: currency.toLowerCase() + ' power', unit: 'P' },
      widget: { id: 'balance', kind: currency === 'Political' ? 'political-power' : 'command-power', trackIds: ['track'],
        countries: { kind: 'tags', tags: [tag] }, time: { mode: 'follow' }, options: { endLabels: false, ceiling: false } },
      runtime: { client: {}, tracks: [{ id: 'track', axisOffset: 0, branch: { id: 'branch', firstHour: 0, lastHour: 30 },
        source: { id: 'source', revision: '1', metadata: { campaignKey: 'synthetic campaign', branches: [], countries: [] } } }],
      clock: { getHour: () => state.clock, getSnapshot: () => ({ playing: false, hoursPerSecond: 24 }) } },
      active: true, headerSlot: null, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => { state.cursor = 0; state.effects = []; renderToStaticMarkup(createElement(Widget, props)); for (const effect of state.effects) effect(); };
    const chart = () => { assert.ok(state.chart); return state.chart; };
    const frame = plotFrame(300, 200, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 30], [0, 20], false);
    const drawn = () => {
      const xs: number[] = [];
      const ctx = { save() {}, restore() {}, beginPath() {}, moveTo(x: number) { xs.push(x); }, lineTo() {}, stroke() {} } as unknown as CanvasRenderingContext2D;
      chart().layers.at(-1)!.draw(ctx, frame, state.tokens, { playhead: state.clock });
      return xs;
    };
    const stops = () => keyStopList(chart().keyHours, [], 0, state.clock)!;
    render(); render();
    assert.deepEqual(drawn(), [], 'a payment concealed by constant balance has no tick before its supplying read');
    assert.deepEqual(stops().map(stop => stop.at), [0], 'an unconfirmed event date supplies no keyboard stop');
    const before = chart().keyHours;
    state.clock = 25.5; render();
    assert.equal(chart().keyHours, before, 'navigation reuses its hours between confirmation thresholds');
    const first = keyedStop('End', null, stops())!;
    assert.equal(first.at, 0);
    chart().onPick!(first.at, { frame, until: state.clock }); render();
    chart().onPick!(first.at, { frame, until: state.clock }); render();
    assert.equal(state.drill!.hour, 0); assert.equal(state.drill!.mode, 'income');
    state.drillHost!.onClose(); render();
    state.clock = 26; render();
    assert.deepEqual(drawn(), [Math.floor(frame.toX(24)) + 0.5]);
    assert.deepEqual(stops().map(stop => stop.at), [0, 24, 26]);
    const payment = keyedStop('ArrowLeft', keyedStop('End', null, stops()), stops())!;
    assert.equal(payment.at, 24);
    chart().onPick!(payment.at, { frame, until: state.clock }); render();
    chart().onPick!(payment.at, { frame, until: state.clock }); render();
    assert.equal(state.drill!.hour, 24); assert.equal(state.drill!.mode, 'spending');
    state.drillHost!.onClose(); render();
    state.clock = 25; render();
    assert.deepEqual(drawn(), []);
    assert.deepEqual(stops().map(stop => stop.at), [0], 'a backward seek removes both the tick and event stop');
  } finally { delete host.__paymentFixture; delete host.__paymentHook; await vite.close(); }
});
