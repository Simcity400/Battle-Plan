import assert from 'node:assert/strict';
import test from 'node:test';
import { drawLineSummary, exactRuns, extentOfSummary } from '../../charts/chartSummary.ts';
import type { ChartSummary, ChartBucket } from '../../charts/chartSummary';
import { drawLine } from '../../charts/draw.ts';
import { plotFrame } from '../../charts/frame.ts';
import { runAt, extentOf } from '../../charts/series.ts';
import type { ChartTokens } from '../../charts/tokens';
import type { AmountRun, EquipmentStockpileReply } from '../../generated/dashboard';
import { chartRuns, stockGroups, shareOf, stockChoices, automaticGroups, variantKey } from './stockpileData.ts';
import { exactStockpileRuns, stockpileSummaries, mergeStockpileReads } from './preparedStockpileChart.ts';
import type { PreparedStockpileReply, StockpileOperand, StockpileView } from './preparedStockpileChart';
import { stockpileParityModel } from './stockpileParityModel.ts';
import { stockpilePartHistory } from './stockpileDrill.ts';

const amount = (firstHour: number, lastHour: number, amountRaw: string | null): AmountRun => ({ firstHour, lastHour, amountRaw });
const original: EquipmentStockpileReply = {
  branchId: '1', campaignKey: 'fixture', tag: 'MOD', decimals: 5, warnings: [], reads: [0], lineReads: [0], logisticsReads: [0, 10, 20, 23, 25, 27],
  stockHours: [{ firstHour: 0, lastHour: 30 }], lineHours: [{ firstHour: 0, lastHour: 30 }], balanceHours: [{ firstHour: 0, lastHour: 30 }], aiHours: [],
  needBoundRaw: String(0x53e2d620fb40), types: [{ key: 'rifle', domain: 'land', name: 'Rifle', needNote: null }],
  variants: [{ id: '10', token: 'rifle_model', archetype: 'rifle', names: [{ firstHour: 0, name: 'Model', known: true, design: 'first' }],
    stock: [amount(0, 30, '9007199254740993')], daily: [], dailyIc: [], unitCostLow: [], unitCostHigh: [] }],
  logistics: [{ key: 'rifle', stock: [amount(0, 30, '7')],
    balance: [amount(0, 9, '1'), amount(10, 19, '2'), amount(20, 22, '4'), amount(23, 24, '8'), amount(25, 26, null), amount(27, 30, '-1')],
    available: [amount(0, 9, '4'), amount(10, 19, '8'), amount(20, 22, '16'), amount(23, 24, '20'), amount(25, 26, '20'), amount(27, 30, '6')] }],
};
const bucket = (firstHour: number, lastHour: number, value: number): ChartBucket => ({ firstHour, lastHour, firstValue: value, lastValue: value,
  lastValueHour: firstHour, minHour: firstHour, minValue: value, maxHour: firstHour, maxValue: value, firstBreak: firstHour === 0, hasBreak: false });

function operand(field: string, id: string, key: string, rows: readonly AmountRun[], from: number, to: number, width: number): StockpileOperand {
  const dates = [...new Set([0, 31, ...rows.flatMap(row => [row.firstHour, row.lastHour + 1])])].sort((a, b) => a - b);
  const presence = dates.slice(0, -1).map((firstHour, index) => {
    const row = runAt(rows, firstHour);
    return { firstHour, lastHour: dates[index + 1] - 1, value: row == null ? 0 : row.amountRaw === null ? -(index + 1) : index + 1, breakBefore: firstHour === 0 };
  });
  const values = presence.flatMap(run => {
    const row = runAt(rows, run.firstHour);
    return row?.amountRaw === null ? [] : [{ ...run, value: Number(row?.amountRaw ?? '0') }];
  });
  const summary = (runs: typeof presence): ChartSummary => ({ from, to, width, buckets: [], runs: runs.filter(run => run.lastHour >= Math.floor(from) && run.firstHour <= to) });
  return { field, id, key, values: summary(values), presence: summary(presence), samples: [], rowSamples: [] };
}
function prepared(from: number, to = from, width = 1): PreparedStockpileReply {
  const visible = (rows: readonly AmountRun[]) => rows.filter(row => row.lastHour >= Math.floor(from) && row.firstHour <= to);
  const values = original.logistics[0], stock = original.variants[0];
  const coverage: ChartSummary = { from, to, width, buckets: [], runs: [{ firstHour: 0, lastHour: 30, value: 0, breakBefore: true }] };
  return { ...original, variants: [{ ...stock, stock: visible(stock.stock) }], logistics: [{ ...values,
    balance: visible(values.balance), stock: visible(values.stock), available: visible(values.available) }],
    prepared: { from, to, stockRecorded: true, linesRecorded: true, balanceRecorded: true, aiRecorded: false,
      coverage: [coverage, coverage, coverage, { ...coverage, runs: [] }], choices: {}, designs: { '10': ['first', 'renamed'] }, nonzeroLogistics: ['rifle'],
      operands: [operand('stock', '10', '10', stock.stock, from, to, width), operand('balance', '1', 'rifle', values.balance, from, to, width),
        operand('logistics-stock', '1', 'rifle', values.stock, from, to, width), operand('available', '1', 'rifle', values.available, from, to, width)] } };
}
const changes = [0, 10, 20, 23, 25, 27, 31];
function commands() {
  const stream: unknown[][] = [];
  const context = new Proxy({}, { get: (_target, key) => (...args: unknown[]) => stream.push([key, ...args]),
    set: (_target, key, value) => { stream.push([key, value]); return true; } }) as CanvasRenderingContext2D;
  return { context, stream };
}

test('exact ratio windows recover coalesced supplying intervals and paint the identical legacy canvas stream', async () => {
  const groups = stockGroups(original, 'type', 30);
  const view: StockpileView = { from: 20.25, to: 29.5, width: 280 };
  let reads = 0;
  const base = prepared(view.from, view.to, view.width);
  const runs = await exactStockpileRuns(base, groups, 'type', view, [0, 30], async hour => { reads++; return prepared(hour); },
    async (hour, forward) => changes.filter(change => forward ? change > hour && change <= 30 : change < hour).sort((a, b) => forward ? a - b : b - a)[0] ?? null);
  const expected = chartRuns(original, groups[0], 'type');
  assert.deepEqual(runs.get('rifle'), expected);
  assert.ok(reads <= 5, 'edge walks retain point reads, not a branch reply');
  const summary = { ...view, buckets: [], runs: runs.get('rifle')! };
  const tokens = { lineWidth: { series: 2 } } as ChartTokens;
  const frame = plotFrame(300, 160, { left: 10, right: 10, top: 10, bottom: 10 }, [view.from, 30.5], [-20, 100], false);
  for (const held of [false, true]) for (const options of [{}, { area: true, dash: [5, 4], endDot: false }]) {
    const legacy = commands(), rebuilt = commands();
    drawLine(legacy.context, frame, tokens, expected, '#123', { ...options, held, until: view.to });
    drawLineSummary(rebuilt.context, frame, tokens, summary, '#123', held, view.to, options);
    assert.deepEqual(rebuilt.stream, legacy.stream);
  }
  assert.deepEqual(extentOfSummary(summary), extentOf(expected, view.from, view.to));
});

test('dense ratio samples use exact integer sums and the legacy division order, retaining unknowns', () => {
  const base = prepared(20, 22_000, 1);
  const n = base.prepared!.operands.find(row => row.field === 'balance')!;
  const a = base.prepared!.operands.find(row => row.field === 'available')!;
  const s = base.prepared!.operands.find(row => row.field === 'logistics-stock')!;
  for (const row of base.prepared!.operands) Object.assign(row, { values: { from: 20, to: 22_000, width: 1, buckets: [bucket(20, 22_000, 0)] },
    presence: { from: 20, to: 22_000, width: 1, buckets: [bucket(20, 22_000, 1)] }, rowSamples: [[1, 1, 1, 1]] });
  Object.assign(n, { samples: [['1', '4', '-2', '4']] }); Object.assign(a, { samples: [['4', '12', '0', '12']] }); Object.assign(s, { samples: [['7', '7', '7', '7']] });
  base.prepared!.coverage.forEach(summary => Object.assign(summary, { from: 20, to: 22_000, width: 1, runs: undefined, buckets: [bucket(20, 22_000, 0)] }));
  const result = stockpileSummaries(base, stockGroups(original, 'type', 30), 'type', { from: 20, to: 22_000, width: 1 }, false).get('rifle')!;
  assert.equal(exactRuns(result), null);
  assert.equal(result.buckets[0]?.firstValue, (1 / 3) * 100);
  assert.equal(result.buckets[0]?.lastValue, 50);
  assert.equal(result.buckets[0]?.minValue, -100);
  Object.assign(a, { rowSamples: [[-1, -1, -1, -1]] });
  base.types = base.types.map(type => ({ ...type, needNote: 'This type requires its matching native available operand.' }));
  assert.equal(stockpileSummaries(base, stockGroups(original, 'type', 30), 'type', { from: 20, to: 22_000, width: 1 }, false).get('rifle')!.buckets[0], null);
});

test('reached reads preserve canonical bounds and saved design witnesses without paging naming history', () => {
  const merged = mergeStockpileReads([prepared(0), prepared(20)]);
  assert.deepEqual(merged.variants[0].stock, original.variants[0].stock);
  const group = stockGroups(merged, 'variant', 20)[0];
  assert.equal(stockChoices([group], 'variant').lines(variantKey(merged, merged.variants[0]) + '#renamed'), 1);
  const copy = prepared(20);
  copy.prepared!.choices = { '["type","rifle"]': { raw: '9007199254740993', stockDrawable: true, shareDrawable: true } };
  assert.deepEqual(automaticGroups(copy, stockGroups(copy, 'type', 20), 'type'), ['rifle']);
  assert.ok(shareOf({ balance: 1n, need: BigInt(original.needBoundRaw) - 1n })! > 0);
});

test('dense types use correlated aggregate operands and the share paired at the same hours', () => {
  const data = prepared(0, 30_000, 1);
  const view = { from: 0, to: 30_000, width: 1 };
  data.prepared!.coverage.forEach(summary => Object.assign(summary, { ...view, runs: undefined, buckets: [bucket(0, 30_000, 0)] }));
  const value = (field: string, samples: (string | null)[]): StockpileOperand => ({ field, id: 'group', key: 'group',
    values: { ...view, buckets: [bucket(0, 30_000, Number(samples[0] ?? 0))] }, presence: { ...view, buckets: [] }, rowSamples: [], samples: [samples] });
  // Balance is least (10) at one read and need least (10) at another: 10% and 1,000% occurred; dividing the minima would invent 100%.
  const paired = { ...bucket(0, 30_000, 100), minHour: 10, minValue: 10, maxHour: 20, maxValue: 1000 };
  data.prepared!.groups = [{ key: '["type","rifle"]', operands: [value('stock', ['100', '100', '100', '100']),
    value('balance', ['100', '100', '10', '100']), value('need', ['100', '100', '10', '100']),
    { ...value('share', [null, null, null, null]), values: { ...view, buckets: [paired] } }] }];
  const group = stockGroups(data, 'type', 30)[0];
  const share = stockpileSummaries(data, [group], 'type', view, false).get(group.key)!;
  assert.deepEqual(share.buckets[0], paired, 'independent operand extrema are never divided');
  assert.equal(exactRuns(share), null);
  const stock = stockpileSummaries(data, [group], 'variant', view, false).get(group.key)!;
  assert.equal(stock.buckets[0]?.maxValue, .001, 'the aggregate is used instead of summing separate model maxima');
});

test('needed overlay reconstructs its original held supplying bounds and legacy command stream', async () => {
  const group = stockGroups(original, 'type', 30)[0];
  const view = { from: 10.5, to: 20.75, width: 280 };
  const rebuilt = await exactStockpileRuns(prepared(view.from, view.to, view.width), [group], 'variant', view, [0, 30],
    async hour => prepared(hour), async (hour, forward) => changes.filter(change => forward ? change > hour && change <= 30 : change < hour)
      .sort((a, b) => forward ? a - b : b - a)[0] ?? null, 'need');
  const expected = stockpilePartHistory(original, group, 'balance').series.find(row => row.id === 'need')!.runs
    .map(run => ({ ...run, value: -run.value })).filter(run => run.lastHour >= view.from && run.firstHour <= view.to);
  assert.deepEqual(rebuilt.get(group.key), expected);
  const frame = plotFrame(300, 160, { left: 10, right: 10, top: 10, bottom: 10 }, [view.from, view.to], [0, .001], false);
  const legacy = commands(), current = commands(), tokens = { lineWidth: { series: 2 } } as ChartTokens;
  drawLine(legacy.context, frame, tokens, expected, '#123', { held: true, dash: [5, 4], until: 20.75 });
  drawLineSummary(current.context, frame, tokens, { ...view, buckets: [], runs: rebuilt.get(group.key)! }, '#123', true, 20.75, { dash: [5, 4] });
  assert.deepEqual(current.stream, legacy.stream);
});

test('named parity model reads the same exact displayed facts from legacy and date replies', () => {
  const hour = 20;
  const current = prepared(hour);
  const label = 'Equipment stockpile exact values';
  assert.deepEqual(stockpileParityModel(current, stockGroups(current, 'type', hour), hour, label),
    stockpileParityModel(original, stockGroups(original, 'type', hour), hour, label));
  assert.equal(stockpileParityModel(current, stockGroups(current, 'type', hour), hour + 1, label).complete, false,
    'a held reply cannot complete a new displayed hour');
});
