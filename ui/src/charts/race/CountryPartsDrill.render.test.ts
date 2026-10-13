import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { tokensFromValues } from '../tokens.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { CountryFactoriesReply, EquipmentProducedReply, ProducedKey } from '../../generated/dashboard';
import { factoryModel, validateFactoriesReply } from '../../widgets/factories/factoriesData.ts';
import { factoryDrillData, factoryDrillNumbers } from '../../widgets/factories/factoryDrill.ts';
import { producedModel, validateProducedReply } from '../../widgets/equipment-produced/producedData.ts';
import { producedDrillData, producedDrillNumbers } from '../../widgets/equipment-produced/producedDrill.ts';

const shared = globalThis as typeof globalThis & { economyDrillReply?: unknown; economyDrillChart?: { onContextMenu: (hour: number, x: number, y: number) => void };
  economyDrillLooks: { hours?: ReadonlyMap<string, number>; exact: boolean }[]; economyDrillRequests: { params: { tags: string[] } }[];
  economyDrillMenu?: { run: () => void }[] };
shared.economyDrillLooks = []; shared.economyDrillRequests = [];
const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
  server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{
    name: 'economy-drill-inputs', enforce: 'pre',
    transform(code, id) {
      const path = id.replaceAll('\\', '/');
      if (path.endsWith('/charts/useChartTokens.ts')) return "export { useChartTokens, useViewportHeight } from 'economy-drill-inputs';";
      if (!path.endsWith('/race/CountryPartsDrill.tsx') && !path.endsWith('/race/CountryRaceView.tsx')) return;
      for (const dependency of ['../../dashboard/products', '../../dashboard/playback', '../useChartTokens',
        '../../dashboard/countryLooks', '../../dashboard/countryFlagsAt', '../../components/ContextMenu', './countryColours']) {
        code = code.replace(`from '${dependency}'`, "from 'economy-drill-inputs'");
      }
      return code.replace("import TimeChart from '../TimeChart';", "import { TimeChart } from 'economy-drill-inputs';");
    },
    resolveId(id) { if (id === 'economy-drill-inputs') return '\0economy-drill-inputs'; },
    load(id) {
      if (id !== '\0economy-drill-inputs') return;
      return `import { createElement } from 'react';
        export const useProduct = (_, request, active, validate) => { if (request) globalThis.economyDrillRequests.push(request);
          return { data: request ? validate(globalThis.economyDrillReply) : undefined, retry() {} }; };
        export const usePlaybackHour = () => {}; export const usePlaybackSpeed = () => 24;
        export const useChartTokens = () => (${JSON.stringify(tokensFromValues(() => '#777', 'sans-serif'))});
        export const useViewportHeight = () => 1000;
        export const useCountryColours = () => () => '#777';
        export const useCountryLooks = (_, track, tags, hour, active, segmentAt, hours, exact) => {
          globalThis.economyDrillLooks.push({ hours, exact }); return { name: tag => tag, flag: () => undefined, color: () => null, revision: 0 }; };
        export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }), revision: 0 });
        export const useContextMenu = () => ({ open: (x, y, items) => { globalThis.economyDrillMenu = items; }, menu: null });
        export const TimeChart = props => { globalThis.economyDrillChart = props; return createElement('div', { 'data-chart': true }); };`;
    },
  }] });
const { default: CountryPartsDrill } = await vite.ssrLoadModule('/ui/src/charts/race/CountryPartsDrill.tsx').finally(() => vite.close()) as typeof import('./CountryPartsDrill.tsx');
const track = { id: 'track', axisOffset: 0, branch: { id: 'branch', firstHour: 0, lastHour: 200 },
  source: { id: 'source', revision: 1, metadata: { countries: [] } } } as unknown as BoundTrack;
const sought: number[] = [];
const runtime = { client: {}, tracks: [track], clock: { getHour: () => 150, getSnapshot: () => ({ playing: false }), getSpeed: () => 24 },
  requestSeek: (_: string, hour: number) => sought.push(hour) } as unknown as WidgetRuntime;
const widget = (tag: string, origin: number): WidgetSpec => ({ id: 'widget', kind: 'equipment-produced', configVersion: 1, title: 'Produced',
  trackIds: ['track'], countries: { kind: 'tags', tags: [tag] }, time: { mode: 'campaign' }, options: { show: 'breakdown', drillOriginHour: origin } });
const key = (token: string): ProducedKey => ({ key: token, name: token, typeKey: token, domain: null, names: [] });
const base: EquipmentProducedReply = { branchId: 'branch', measure: 'total', count: 'units', decimals: 0, grouping: 'type', types: [], warnings: [],
  keys: [key('Known'), key('Correction'), key('Zero'), key('Unknown')], reads: [8, 32], observations: [8], known: [],
  series: [{ tag: 'MOD', identities: [{ firstHour: 0, lastHour: 127, slot: 3, countryId: 9 }],
    firstHours: [8, 32], lastHours: [31, 127], breaks: [0], membership: null,
    values: [{ key: 0, values: { from: 0, deltas: [12], unknown: [] } }, { key: 1, values: { from: 0, deltas: [-2], unknown: [] } },
      { key: 3, values: { from: 0, deltas: [], unknown: [0, 2] } }], total: { from: 0, deltas: [], unknown: [0, 2] } }] };
function produced(reply: EquipmentProducedReply, tag = 'MOD', origin = 8, hour = 40) {
  shared.economyDrillReply = reply;
  return renderToStaticMarkup(createElement(CountryPartsDrill<EquipmentProducedReply>, {
    widget: widget(tag, origin), runtime, track, active: true, onDiagnostics() {}, options: { show: 'breakdown' },
    extent: [0, 200], moment: hour, playhead: null, product: 'equipment-produced', params: country => ({ tags: [country], top: null }),
    validate: validateProducedReply, decode: value => producedDrillData(producedModel(value)), numbers: producedDrillNumbers,
    defaultTag: tag, pickTag: () => tag, title: 'Produced parts', measure: 'Running total', virtual: true, positiveGeometry: true,
    warnings: [], resetKey: 'source/branch/selection', main: () => null,
  }));
}

test('all-unknown totals retain full known/correction/zero/unknown rows, status and Follow controls with blank geometry', () => {
  const html = produced(base);
  for (const text of ['Known', '>12<', 'Correction', '>-2<', 'Zero', 'Unknown', 'Not known', 'Follow']) assert.ok(html.includes(text), text);
  assert.match(html, /Breakdown unavailable for MOD/);
  assert.equal(/<small>[^<]*%/.test(html), false);
});

test('a restored native origin selects the renamed holder and displayed wide residual shares stay exact', () => {
  const wide = { ...base, keys: [key('Wide correction'), key('Small correction')], series: [{ ...base.series[0],
    values: [{ key: 0, values: { from: 0, deltas: [-Number.MAX_SAFE_INTEGER], unknown: [] } },
      { key: 1, values: { from: 0, deltas: [-1], unknown: [] } }], total: { from: 0, deltas: [1], unknown: [] } }] };
  assert.match(produced(wide), /<small>900719925474099300%<\/small>/);
  const renamed: EquipmentProducedReply = { ...base, keys: [key('Native design')], reads: [10, 11, 13], series: [
    { ...base.series[0], tag: 'OLD', identities: [{ firstHour: 10, lastHour: 10, slot: 3, countryId: 9 },
      { firstHour: 13, lastHour: 20, slot: 8, countryId: 99 }], firstHours: [13], lastHours: [20], breaks: [0],
      values: [{ key: 0, values: { from: 0, deltas: [99], unknown: [] } }], total: { from: 0, deltas: [99], unknown: [] } },
    { ...base.series[0], tag: 'NEW', identities: [{ firstHour: 11, lastHour: 20, slot: 3, countryId: 9 }], firstHours: [11], lastHours: [20], breaks: [0],
      values: [{ key: 0, values: { from: 0, deltas: [12], unknown: [] } }], total: { from: 0, deltas: [12], unknown: [] } },
  ] };
  const html = produced(renamed, 'OLD', 10, 19);
  assert.ok(html.includes('NEW'));
  assert.ok(html.includes('>12<'));
  assert.equal(html.includes('>99<'), false);
  assert.deepEqual(shared.economyDrillRequests.at(-1)?.params.tags, ['OLD']);
});

test('actual factory drill appearance and Jump use the carried read while an equal fresh confirmation advances it', () => {
  const reply: CountryFactoriesReply = { branchId: 'branch', warnings: [], fields: [{ group: 'civilian', part: 'total' }],
    groups: [{ key: 'civilian', flag: 'mod factory flag', buildings: [{ token: 'mod works', name: 'Recorded works' }] }],
    series: [{ tag: 'OLD', identities: [{ firstHour: 100, lastHour: 102, slot: 3, countryId: 9 }], membership: null,
      segments: [{ firstHour: 100, lastHour: 102 }], runs: [{ firstHour: 100, lastHour: 102, values: [12], startsSegment: true }],
      reads: [{ firstHour: 100, lastHour: 100, readHour: 100, hourly: true }, { firstHour: 101, lastHour: 101, readHour: 100, hourly: false },
        { firstHour: 102, lastHour: 102, readHour: 102, hourly: true }] }] };
  shared.economyDrillReply = reply;
  for (const [hour, read] of [[101, 100], [102, 102]]) {
    renderToStaticMarkup(createElement(CountryPartsDrill<CountryFactoriesReply>, { widget: widget('OLD', 100), runtime, track, active: true,
      onDiagnostics() {}, options: { show: 'breakdown' }, extent: [100, 102], playhead: null, moment: hour, product: 'country-factories',
      params: tag => ({ tags: [tag], top: null }), validate: validateFactoriesReply, decode: value => factoryDrillData(factoryModel(value), ['civilian'], 'built'),
      numbers: factoryDrillNumbers, defaultTag: 'OLD', pickTag: () => 'OLD', title: 'Factory types', measure: 'Built', warnings: [], resetKey: 'factory', main: () => null }));
    assert.equal(shared.economyDrillLooks.at(-1)?.hours?.get('OLD'), read);
    assert.equal(shared.economyDrillLooks.at(-1)?.exact, true);
    shared.economyDrillChart!.onContextMenu(hour, 0, 0);
    shared.economyDrillMenu![0].run();
    assert.equal(sought.at(-1), read);
  }
});
