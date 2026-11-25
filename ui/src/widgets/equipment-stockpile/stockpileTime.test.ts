import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { ChartBucket } from '../../charts/chartSummary.ts';
import type { TiledLine } from '../../dashboard/tiles/view';
import { setToken } from '../../dashboard/options.ts';
import { boundTiledSeries, createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import type { FamilyPlanContext, PixelBinnedSourceRead, PointSourceRead } from '../../families/plan.ts';
import type { TimeChartProjection, TimeChartWidgetDeclaration } from '../../families/time-chart/declaration.ts';
import { knownValue } from '../../families/value.ts';
import type { StockpileLinesReply } from '../../generated/dashboard';
import { TIME_CHART_VIEWS } from '../manifest/time-chart.ts';

const FIRST = 60_000_000;
const HOUR = FIRST + 400;

async function view() {
  const entry = TIME_CHART_VIEWS.find(item => item.kind === 'equipment-stockpile' && item.view === 'time');
  assert.ok(entry, 'the Time view is a time-chart manifest entry');
  const spec = await entry.loadSpec();
  return { projection: spec.projection as TimeChartProjection<unknown>, declaration: spec.declaration as TimeChartWidgetDeclaration<unknown> };
}

function context(options: Record<string, unknown>): FamilyPlanContext {
  const branch = { id: '7', firstHour: FIRST, lastHour: FIRST + 1000 };
  return {
    track: { source: { id: 'src', revision: '1' }, axisOffset: 0, branch },
    branch,
    lastRecordedHour: FIRST + 1000,
    options,
    following: true,
    aux: undefined,
    countryTag: 'GER',
    countryName: 'Germany',
    countries: { tags: ['GER'], membership: { GER: [[FIRST, FIRST + 1000]] } },
    gateSpan: (firstHour: number, lastHour: number) => ({ firstHour, lastHour }),
  } as unknown as FamilyPlanContext;
}

function bucket(firstHour: number, lastHour: number, value: number): ChartBucket {
  return { firstHour, lastHour, firstValue: value, lastValue: value, lastValueHour: lastHour, minHour: firstHour, minValue: value,
    maxHour: firstHour, maxValue: value, firstBreak: false, hasBreak: false };
}

function tiled(tag: string, value: number): TiledLine {
  return { tag, head: 'h', decimals: 5, summary: { from: FIRST, to: HOUR, width: 1, buckets: [bucket(FIRST, HOUR, value)], tiles: { binHours: 96, valueAt: () => null } },
    missing: [], parts: [], marks: null };
}

function reply(hour: number, values: Record<string, number | null>, top: StockpileLinesReply['top']): StockpileLinesReply {
  const keys = Object.keys(values);
  return {
    branchId: '7', hour, measure: 'stock', decimals: 5, nextChange: hour + 24, warnings: [], top,
    lines: keys.map((key, ordinal) => ({ key, name: key === 'rifles' ? 'Infantry equipment' : null, domain: 'land', ordinal,
      value: values[key] ?? null, missing: values[key] == null ? 'The stockpile was not read at this hour.' : null })),
  };
}

test('the Time view asks the product for the main types at the date, and a chosen list by name', async () => {
  const { declaration } = await view();
  const automatic = declaration.sources(context({}), { width: 600, height: 300 } as never);
  const tiles = automatic.entries.find(entry => entry.name === 'stock')!.read as PixelBinnedSourceRead;
  assert.deepEqual(tiles.top, { n: 5 });
  assert.deepEqual(tiles.tags, []);
  assert.deepEqual(tiles.family, { kind: 'equipment-stockpile', measure: 'stock', country: 'GER', included: [], membership: [{ firstHour: FIRST, lastHour: FIRST + 1000 }] });
  const lines = automatic.entries.find(entry => entry.name === 'lines')!.read as PointSourceRead;
  assert.equal(lines.product, 'equipment-stockpile-lines');
  assert.equal(lines.params?.top, 5);
  assert.deepEqual(lines.provenInterval?.({ nextChange: HOUR + 24 }, HOUR), [HOUR, HOUR + 23]);
  assert.deepEqual(lines.provenInterval?.({ nextChange: null }, HOUR), [HOUR, Infinity]);
  const chosen = declaration.sources(context({ chosen: `${setToken('mod#tanks')} ${setToken('rifles')}`, measure: 'share' }), { width: 600, height: 300 } as never);
  const chosenTiles = chosen.entries.find(entry => entry.name === 'stock')!.read as PixelBinnedSourceRead;
  assert.equal(chosenTiles.top, undefined);
  assert.deepEqual(chosenTiles.tags, ['mod#tanks', 'rifles'], 'a quoted type key keeps its #');
  assert.equal((chosenTiles.family as { measure: string }).measure, 'share');
  assert.equal((chosen.entries.find(entry => entry.name === 'lines')!.read as PointSourceRead).params?.top, undefined);
});

test('the Time view draws the product’s main types in its order, missing as missing, and hovers the drawn types at the hit hour', async () => {
  const { declaration, projection } = await view();
  const ctx = context({ endLabels: true });
  const options = declaration.projectionOptions(ctx, { extent: [FIRST, HOUR], surface: '#111111' });
  const hoverHour = FIRST + 100;
  const readView = createBoundedReadView({
    hour: HOUR,
    series: new Map([['stock', new Map([['rifles', boundTiledSeries(tiled('rifles', 900), displayedHour(HOUR), { headValue: knownValue(900) })]])]]),
    panels: new Map([['lines', reply(HOUR, { guns: 50, rifles: 900, tanks: null }, { keys: ['rifles', 'tanks'], refused: null })]]),
    hovers: new Map([['hover', { hour: hoverHour, data: reply(hoverHour, { guns: 70, rifles: 400, tanks: 3 }, null) }]]),
  });
  const model = projection.project(readView, options);
  assert.deepEqual(model.series.map(series => series.key), ['rifles', 'tanks']);
  assert.equal(model.series[0]!.name, 'Infantry equipment');
  assert.equal(model.series[0]!.endLabel?.value, '900');
  assert.equal(model.series[1]!.name, 'tanks');
  assert.equal(model.series[1]!.headValue, null);
  assert.equal(model.series[1]!.absentReason, 'The stockpile was not read at this hour.');
  assert.equal(model.series[1]!.endLabel, null);
  assert.notEqual(model.series[0]!.color, model.series[1]!.color);
  const hover = model.hoverDetail!;
  assert.equal(hover.hour, hoverHour);
  assert.deepEqual(hover.items.slice(1).map(item => [item.label, item.value]), [['Infantry equipment', '400'], ['tanks', '3']]);
  assert.deepEqual(model.auditRows?.map(row => row.value), ['900', 'The stockpile was not read at this hour.']);
});

test('a refused ranking draws no types and says why; the settings reach the axis and legend', async () => {
  const { declaration, projection } = await view();
  const refused = 'The main types are not ranked: the stock of every type is needed.';
  const readView = createBoundedReadView({
    hour: HOUR,
    panels: new Map([['lines', reply(HOUR, { rifles: 900 }, { keys: [], refused })]]),
  });
  const automatic = projection.project(readView, declaration.projectionOptions(context({}), { extent: [FIRST, HOUR], surface: '#111111' }));
  assert.equal(automatic.series.length, 0);
  assert.equal(automatic.empty?.detail, refused);
  const set = declaration.projectionOptions(context({ chosen: setToken('rifles'), measure: 'share', endLabels: false, yMin: -50, log: false }), { extent: [FIRST, HOUR], surface: '#111111' });
  const chosen = projection.project(readView, set);
  assert.deepEqual(chosen.series.map(series => series.key), ['rifles']);
  assert.equal(chosen.yAxis.format, 'percent');
  assert.equal(chosen.yAxis.min, -50);
  assert.deepEqual(chosen.legend?.map(entry => entry.label), ['Infantry equipment']);
  assert.equal(chosen.series[0]!.endLabel, null);
});
