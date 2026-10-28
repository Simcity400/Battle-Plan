import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, type ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { dayLabel } from '../../charts/draw.ts';
import type { CountryRaceViewProps } from '../../charts/race/CountryRaceView';
import { mountModule } from '../../dashboard/pendingGroups.fixture.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import type { ProductionSelection, ProductionSelectionRequest } from './preparedProducedData';

const BRANCH = { id: '1', firstHour: 0, lastHour: 2000 };
const OBSERVATION = 480;
const page = (fields: Partial<ProductionSelection> = {}): ProductionSelection => ({ schema: 1, branchId: '1', firstHour: 1990, lastHour: 2000, known: true, recorded: true,
  observation: OBSERVATION, keys: [{ key: '7', name: null, typeKey: 'gun', domain: 'land', names: [] }], types: [{ key: 'gun', name: 'Gun', domain: 'land' }],
  totalCountries: 1, offset: '0', valueMin: '0', valueMax: '5', warnings: [],
  countries: [{ tag: 'A', slot: 1, countryId: 2, segment: '1', total: '5', equipmentTotal: '5', readHour: 1990, observation: OBSERVATION,
    values: [{ key: '7', amount: '5', equipmentAmount: '5' }] }], ...fields });

interface Answer { data?: ProductionSelection; hour?: number; pending: boolean; unavailable?: boolean; error?: string }
interface Fixture { answer: (request: ProductionSelectionRequest) => Answer; prepared: ProductionSelectionRequest[]; history: unknown[]; view?: CountryRaceViewProps; tag: string | null }
const host = globalThis as typeof globalThis & { __producedPath?: Fixture };
const replacements: Record<string, string> = {
  '/widgets/equipment-produced/usePreparedProduction.ts': `export const hasPreparedProduction = () => true;
    export const usePreparedProduction = (_client, _source, request) => { if (!request) return { pending: false, retry() {} };
      globalThis.__producedPath.prepared.push(request); return { ...globalThis.__producedPath.answer(request), retry() {} }; };`,
  '/dashboard/products.ts': `export const useProduct = (_client, request) => { if (request) globalThis.__producedPath.history.push(request); return { retry() {} }; };
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
  '/dashboard/playback.ts': 'export const usePlaybackHour = () => 2000;',
  '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ retry() {} });',
  '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 480; export const useChartTokens = () => ({ categories: ["red", "blue"], muted: "grey" });',
  '/widgets/equipment-stockpile/useStockpileArtwork.ts': 'export const useStockpileArtwork = () => undefined;',
  '/charts/race/CountryPartsDrill.tsx': `import { createElement } from 'react'; export default props => createElement('div', null, props.main(() => {}, true),
    props.details({ tag: 'A', hour: 1995, tagAt: () => globalThis.__producedPath.tag, key: [] }));`,
  '/charts/race/CountryRaceView.tsx': `import { createElement } from 'react'; export default props => { globalThis.__producedPath.view = props; return createElement('div'); };`,
};
const widgetProps = { active: true, widget: { kind: 'equipment-produced', options: { view: 'race', measure: 'total' }, trackIds: ['track'], time: { mode: 'campaign' },
  countries: { kind: 'group', group: { type: 'top', count: 5 } } },
  runtime: { client: {}, clock: { getHour: () => 2000 }, tracks: [{ id: 'track', selectedTag: 'A', axisOffset: 0,
    source: { id: 'source', revision: 'revision', metadata: { countries: [] } }, branch: BRANCH }] } } as unknown as WidgetProps;

test('the prepared race chooses its type, dates its caption and reads its breakdown from prepared pages alone', async () => {
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/equipment-produced/PreparedEquipmentProducedWidget.tsx', replacements);
  try {
    const run = (automatic: Partial<ProductionSelection>) => {
      const fixture: Fixture = { prepared: [], history: [], tag: 'A', answer: request => request.limit === 0 && !request.ranked
        ? { data: page({ countries: [], typeTotals: [['rifle', '9'], ['gun', '12']], ...automatic }), pending: false }
        : { data: page(), hour: request.hour, pending: false } };
      host.__producedPath = fixture;
      const markup = renderToStaticMarkup(createElement(module.default, widgetProps));
      return { fixture, markup };
    };
    const provisional = run({ coverage: { firstHour: 0, lastHour: 1000, complete: false }, pending: [{ firstHour: 1001, lastHour: 2000 }] });
    assert.deepEqual(provisional.fixture.history, [], 'no whole-branch defaults, caption or breakdown request');
    const automatic = provisional.fixture.prepared.find(request => request.limit === 0 && !request.ranked)!;
    assert.equal(automatic.hour, BRANCH.lastHour);
    assert.deepEqual(provisional.fixture.prepared.find(request => request.ranked)?.selection.included, ['gun'], 'the most delivered type is shown');
    assert.match(provisional.fixture.view!.label, /approximate type choice: Gun/);
    assert.ok(provisional.fixture.view!.label.includes(`from ${dayLabel(OBSERVATION)} to`), 'the caption counts from the page observation');
    assert.ok(provisional.fixture.prepared.some(request => request.limit === 1 && request.hour === 1995), 'the breakdown reads its hour');
    const final = run({});
    assert.doesNotMatch(final.fixture.view!.label, /approximate type choice/, 'a complete page chooses for good');
    assert.deepEqual(final.fixture.history, []);
    const none = run({ typeTotals: [] });
    const state = (shown: { fixture: Fixture }) => renderToStaticMarkup(shown.fixture.view!.state as ReactElement);
    assert.match(state(none), /No production data for these countries\./);
    const preparing = run({ typeTotals: [], coverage: { firstHour: 0, lastHour: 1000, complete: false }, pending: [{ firstHour: 1001, lastHour: 2000 }] });
    assert.match(state(preparing), /Most delivered type is still being prepared\./);
  } finally { delete host.__producedPath; await close(); }
});

test('an empty page of unknown amounts stays prepared once the branch has read production', async () => {
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/equipment-produced/PreparedEquipmentProducedWidget.tsx', replacements);
  try {
    const daily = { ...widgetProps, widget: { ...widgetProps.widget, options: { view: 'race', measure: 'day' } } } as WidgetProps;
    const run = (recorded: boolean) => {
      const fixture: Fixture = { prepared: [], history: [], tag: 'A', answer: request => request.limit === 0 && !request.ranked
        ? { data: page({ countries: [], typeTotals: [['gun', '12']] }), pending: false }
        : request.ranked ? { data: page({ countries: [], totalCountries: 0, known: false, recorded }), hour: request.hour, pending: false }
          : { data: page(), hour: request.hour, pending: false } };
      host.__producedPath = fixture;
      renderToStaticMarkup(createElement(module.default, daily));
      return fixture;
    };
    const read = run(true);
    assert.deepEqual(read.history, [], 'no whole-branch history for a day not yet known');
    assert.equal(read.view!.state, null);
    assert.equal(read.view!.empty, 'A full day of production data is unavailable.');
    assert.equal(run(false).history.length, 1, 'only history tells whether production is in the recording');
  } finally { delete host.__producedPath; await close(); }
});

test('the breakdown table reads its hour from a page only where the page answers it exactly', async () => {
  const { module, close } = await mountModule<{ default: (props: Record<string, unknown>) => ReactElement }>('/ui/src/widgets/equipment-produced/ProducedDrillTable.tsx', replacements);
  try {
    const track = (widgetProps.runtime as unknown as { tracks: unknown[] }).tracks[0];
    const render = (answer: (request: ProductionSelectionRequest) => Answer, tag: string | null = 'A') => {
      const fixture: Fixture = { prepared: [], history: [], tag, answer };
      host.__producedPath = fixture;
      const markup = renderToStaticMarkup(createElement(module.default, { runtime: widgetProps.runtime, track, active: true, tag: 'A', hour: 1995,
        tagAt: () => tag, narrow: 'hide', measure: 'total', refits: false, count: 'units', chartType: null, chartTitle: 'Chart', chartKey: [], onChart() {}, prepared: true }));
      return { fixture, markup };
    };
    const answered = render(request => ({ data: page(), hour: request.hour, pending: false }));
    assert.deepEqual(answered.fixture.history, []);
    assert.deepEqual(answered.fixture.prepared.map(request => [request.hour, request.selection.tags, request.selection.count]), [[1995, ['A'], 'units'], [1995, ['A'], 'ic']]);
    assert.doesNotMatch(answered.markup, /Loading what this country made/);
    const fallbacks: [string, (request: ProductionSelectionRequest) => Answer][] = [
      ['an earlier prefix hour', () => ({ data: page({ pending: [{ firstHour: 1991, lastHour: 2000 }], coverage: { firstHour: 0, lastHour: 1990, complete: false } }), hour: 1990, pending: false })],
      ['an unknown amount', request => ({ data: page({ countries: [{ ...page().countries[0], values: [{ key: '7', amount: null, equipmentAmount: null }] }] }), hour: request.hour, pending: false })],
      ['an empty page of unknown amounts', request => ({ data: page({ countries: [], totalCountries: 0, known: false }), hour: request.hour, pending: false })],
      ['unavailable preparation', () => ({ pending: false, unavailable: true })],
      ['approximate values', request => ({ data: page({ coverage: { firstHour: 0, lastHour: 2000, complete: false }, pending: [], approximate: [{ firstHour: 1990, lastHour: 2000 }] }), hour: request.hour, pending: false })],
    ];
    for (const [why, answer] of fallbacks) {
      const { fixture } = render(answer);
      assert.deepEqual(fixture.history.map(request => (request as { params: { tags: string[] } }).params.tags), [['A'], ['A']], why);
    }
    const gone = render(() => ({ pending: false, unavailable: true }), null);
    assert.deepEqual(gone.fixture.history, [], 'a country missing at the hour asks for nothing');
    assert.match(gone.markup, /Country unavailable at this time\./);
  } finally { delete host.__producedPath; await close(); }
});
