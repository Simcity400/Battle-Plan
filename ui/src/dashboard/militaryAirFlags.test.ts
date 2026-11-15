import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { ButterflyItem, ButterflyProps } from '../charts/butterfly';
import { tokensFromValues } from '../charts/tokens.ts';
import type { WidgetProps } from './definitions';
import type { LedgerRow } from './militaryAirLedger';
import { validateMilitaryAirReply } from './militaryAirLedger.ts';
import { OPTIONS } from '../widgets/bombing-damage/definition.ts';
import { partOf } from './militaryAirParts.fixture.ts';
import type { Leaf } from './militaryAirParts.fixture';
import type { MilitaryAirLedger } from './militaryAirLedger';

type Item = ButterflyItem & { row: LedgerRow };
const span = (firstHour: number, lastHour = firstHour) => ({ firstHour, lastHour });
const data: MilitaryAirLedger = { family: 'bombing', firstHour: 10, lastHour: 12, available: true,
  observed: [span(10, 12)], incomplete: [], countries: [
    { tag: 'OLD', identity: '3:9', present: [span(10)], membership: null },
    { tag: 'NEW', identity: '3:9', present: [span(11, 12)], membership: null },
  ], parts: ([
    { tag: 'NEW', identity: '3:9', dealt: false, opponent: null, region: null, buildingId: 7, buildingToken: 'mod works', state: 40001, cause: 0,
      points: [[10, 11, '50000000'], [11, 12, '-10000000']] },
    { tag: null, identity: null, dealt: true, opponent: null, region: null, buildingId: 7, buildingToken: 'mod works', state: 40001, cause: 0,
      points: [[10, 11, '50000000']] },
  ] as Leaf[]).map(partOf), regions: [], buildings: [], warnings: [] };

test('actual bombing adapter forwards dated native flag status without labelling an unknown attacker', async () => {
  validateMilitaryAirReply(data);
  const track = { id: 'track', axisOffset: 0, source: { id: 'source', revision: 1, metadata: { countries: [], campaignKey: 'campaign' } },
    branch: { id: 'branch', firstHour: 10, lastHour: 12, gaps: [] } };
  const state = { data, track, chart: undefined as ButterflyProps<Item> | undefined,
    wanted: [] as [string, number][], exact: false };
  const host = globalThis as typeof globalThis & { __militaryFlags?: typeof state };
  host.__militaryFlags = state;
  // Leave the real view's holder joins, event/publication visibility and exact
  // hover formatting in place; only asynchronous operands and the canvas are stubbed.
  const replacements: Record<string, string> = {
    '/dashboard/useMilitaryAirLedger.ts': 'export const useMilitaryAirLedger = () => ({ data: globalThis.__militaryFlags.data, track: globalThis.__militaryFlags.track, retry() {} });',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackMotion = () => 24;',
    '/dashboard/countryLooks.ts': `export const useCountryLooks = (_client, _track, _tags, _hour, _active, _segment, dates, exact) => {
      globalThis.__militaryFlags.wanted = [...dates]; globalThis.__militaryFlags.exact = exact;
      return { name: (tag, hour) => tag + ' at ' + hour, color: () => null,
        flag: (tag, hour) => hour === 10 ? 'old flag' : undefined,
        flagReason: (tag, hour) => hour === 11 ? 'Flag refused at read 11' : undefined,
        flagPending: (tag, hour) => hour === 12, revision: 0 }; };`,
    '/charts/race/countryColours.ts': 'export const useCountryColours = () => () => "#777777";',
    '/charts/race/CountryRaceView.tsx': 'export default () => null;',
    '/dashboard/BombingDrill.tsx': 'export default () => null;',
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
    '/charts/useChartTokens.ts': `export const useChartTokens = () => (${JSON.stringify(tokensFromValues(() => '#777', 'sans-serif'))});`,
    '/charts/butterfly/index.ts': `import { createElement } from 'react'; export const ButterflyChart = props => {
      globalThis.__militaryFlags.chart = props; return createElement('canvas'); };`,
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'military-country-flags', enforce: 'pre',
      transform(_source, id) { const path = id.replaceAll('\\', '/'); return Object.entries(replacements).find(([suffix]) => path.endsWith(suffix))?.[1]; },
    }] });
  try {
    const View = (await vite.ssrLoadModule('/ui/src/dashboard/MilitaryAirView.tsx')).default as ComponentType<WidgetProps & {
      family: 'bombing'; fields: typeof OPTIONS; title: string;
    }>;
    const props = { widget: { id: 'widget', kind: 'bombing-damage', configVersion: 1, trackIds: ['track'], countries: { kind: 'all' },
      time: { mode: 'pin', windows: { track: { firstOffset: 0, lastOffset: 0 } } }, options: {} },
      runtime: { tracks: [track], client: {}, clock: { getHour: () => 12 } }, active: true, onDiagnostics() {},
      family: 'bombing', fields: OPTIONS, title: 'Bombing damage' } as unknown as WidgetProps & { family: 'bombing'; fields: typeof OPTIONS; title: string };
    const capturedChart = () => { assert.ok(state.chart); return state.chart; };
    const render = (hour: number) => {
      props.widget.time = { mode: 'pin', windows: { track: { firstOffset: hour - 10, lastOffset: hour - 10 } } };
      state.chart = undefined;
      const html = renderToStaticMarkup(createElement(View, props));
      const chart = capturedChart();
      assert.equal(chart.items.length, 1, 'unknown raw actors do not become country items');
      assert.match(html, /Unattributed: /, 'unknown attackers are summed on one plain line');
      return chart;
    };
    const before = render(10), first = before.items[0];
    assert.equal(first.row.tag, 'OLD'); assert.equal(first.row.taken, 0n, 'the future supplied event is not visible yet');
    assert.deepEqual(first.countryFlag, { flag: 'old flag', flagReason: undefined, flagPending: false });
    const supplied = render(11), native = supplied.items[0], tip = supplied.tooltip!(native)!;
    assert.deepEqual(state.wanted, [['NEW', 11]]); assert.equal(state.exact, true);
    assert.equal(native.row.identity, '3:9'); assert.equal(native.label, 'NEW at 11');
    assert.deepEqual(native.countryFlag, { flag: undefined, flagReason: 'Flag refused at read 11', flagPending: false });
    assert.equal(tip.countryFlag, native.countryFlag);
    assert.equal(tip.rows.some(row => /supplied|Read/.test(row.label)), false, 'no recording-time rows in the hover');
    const correction = render(12).items[0];
    assert.equal(correction.row.taken, -10000000n); assert.equal(correction.leftText, '−1');
    assert.deepEqual(correction.countryFlag, { flag: undefined, flagReason: undefined, flagPending: true });
    assert.equal(tip.countryFlag?.flagReason, 'Flag refused at read 11', 'later metadata cannot backfill the pinned hover');
    assert.equal(first.countryFlag?.flag, 'old flag');
  } finally { await vite.close(); delete host.__militaryFlags; }
});
