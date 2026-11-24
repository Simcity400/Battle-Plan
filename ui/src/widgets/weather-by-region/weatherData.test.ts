import test from 'node:test';
import assert from 'node:assert/strict';
import type { DashboardClient, ProductRequest } from '../../dashboard/client';
import { loadProduct, peekProduct, productKey } from '../../dashboard/products.ts';
import {
  columnTooltip, modifierText, selectWeatherReplies, shownRegions, temperatureText, uncoveredSpans, validateProvinces, validateWeather,
  weatherCatalogRegions, weatherHistoryPeriods, weatherLoadingSpans, weatherPeriods, weatherRows, weatherScale,
  weatherTail, weatherTailPeriod, weatherTailWindow, weatherWindow,
} from './weatherData.ts';
import type { WeatherFallback, WeatherReply } from './weatherData';
import type { RegionalWeatherColumns as WeatherColumns } from '../../generated/dashboard';

const colours = { flag: (index: number) => `c${index}`, ramp: (value: number, highest: number) => `r${value / highest}`, clear: 'surface', gap: 'gap' };
const days: WeatherColumns = { days: 1, columns: [{ firstHour: 0, lastHour: 23, sampledHour: 23, readHour: 23, recorded: 1 },
  { firstHour: 24, lastHour: 47, sampledHour: 47, readHour: 47, recorded: 1 }, { firstHour: 48, lastHour: 71, sampledHour: null, readHour: null, recorded: 0 }], regions: [
  { id: '1', observed: [1, 1, 0], clear: [0, 0, 0], measures: [{ id: 'rain', days: [1, 1, 0] }, { id: 'snow', days: [0, 1, 0] },
    { id: 'mod:air', days: [1, 0, 0], values: [['-25000', '-25000', '-25000'], null, null] }] },
  { id: '2', observed: [1, 1, 0], clear: [1, 1, 0], measures: [] }] };
const reply = validateWeather({ firstHour: 0, lastHour: 71, dayHours: 24, regions: [{ id: '1', name: 'Alps' }, { id: '2', name: 'Sea' }, { id: '3', name: 'Desert' }],
  flags: [{ id: 'rain', name: 'Rain' }, { id: 'snow', name: 'Snow' }], modifiers: [{ id: 'air', name: 'Air detection' }],
  temperature: 'Temperature unavailable: no weather settings', columns: days, warnings: [] });

test('overlapping flags all show as stripes, none set is blank, a region never sampled is unknown', () => {
  const periods = weatherPeriods(days, [0, 71], 24);
  assert.deepEqual(periods.map(day => day.readHour), [23, 47, null]);
  const { rows } = weatherRows(reply, reply.regions, periods, 'conditions', null, colours);
  assert.deepEqual(rows[0].cells.map(cell => cell.parts.map(part => part.id)), [['rain'], ['rain', 'snow'], []]);
  assert.deepEqual(rows[0].cells[1].parts.map(part => part.share), [0.5, 0.5]);
  assert.deepEqual(rows[1].cells[0].parts, []);
  assert.equal(rows[2].cells[0].unknown, true);
});

test('a folded column sizes its stripes by days, keeps clear days and never takes an unsampled day for clear', () => {
  const week: WeatherColumns = { days: 7, columns: [{ firstHour: 0, lastHour: 167, sampledHour: 167, readHour: 160, recorded: 6 }],
    regions: [{ id: '1', observed: [6], clear: [3], measures: [{ id: 'rain', days: [2] }, { id: 'snow', days: [1] }] }, { id: '2', observed: [6], clear: [6], measures: [] }] };
  const data = validateWeather({ ...reply, columns: week });
  const { rows } = weatherRows(data, data.regions, weatherPeriods(week, [24, 100], 24), 'conditions', null, colours);
  assert.deepEqual(rows[0].cells[0].parts.map(part => [part.id, part.share, part.color]),
    [['rain', 2 / 7, 'c0'], ['snow', 1 / 7, 'c1'], ['clear', 3 / 7, 'surface'], ['unread', 1 / 7, 'gap']]);
  assert.deepEqual(weatherPeriods(week, [24, 100], 24), [{ firstHour: 24, lastHour: 100, readHour: 160, column: 0, exact: false }]);
  const tip = columnTooltip(data, data.regions[0], 0, 'conditions', null, 'date', hour => `h${hour}`);
  assert.deepEqual(tip.rows.slice(0, 2), [{ label: 'h0 to h167' }, { label: 'Recorded days', value: '6 of 7' }]);
  assert.ok(tip.rows.some(row => row.label === 'Conditions not recorded' && row.value === '1 day'));
  const omitted = validateWeather({ ...data, columns: { ...week, regions: [{ ...week.regions[0], observed: [4], clear: [1] }, week.regions[1]] } });
  assert.deepEqual(columnTooltip(omitted, omitted.regions[0], 0, 'conditions', null, 'date', hour => `h${hour}`).rows[1], { label: 'Recorded days', value: '4 of 7' },
    'a region left out of a day counts only the days it was recorded, never the table’s');
  const unread = columnTooltip(omitted, omitted.regions[0], 0, 'temperature', null, 'date', hour => `h${hour}`);
  assert.deepEqual(unread.rows.slice(1), [{ label: 'Recorded days', value: '0 of 7' }, { label: 'Not recorded' }], 'a measure the region never had is not recorded');
});

test('a modifier is its sampled value; a day without one is unknown, never zero; temperature without an average is blank', () => {
  const periods = weatherPeriods(days, [0, 71], 24);
  const modifier = weatherRows(reply, reply.regions, periods, 'modifiers', 'air', colours).rows;
  assert.deepEqual(modifier[0].cells[0].parts.map(part => part.label), ['-0.25']);
  assert.equal(modifier[0].cells[1].unknown, true);
  assert.equal(modifierText('-25000'), '−25%');
  const temperature = weatherRows(reply, reply.regions, periods, 'temperature', null, colours).rows;
  assert.deepEqual(temperature[0].cells[0], { parts: [] }, 'no temperature measure: no average shown');
  assert.equal(temperatureText('-150000'), '−1.5 °C');
});

test('the asked window keeps whole blocks while days fit, else asks its own days folded to the pixels; regions filter by choice', () => {
  const branch = { firstHour: 0, lastHour: 100_000 };
  assert.deepEqual(weatherWindow(branch, [2_200, 2_300], 400), { first: 2_160, last: 4_319, width: 90 });
  assert.deepEqual(weatherTailWindow(branch, [2_200, 2_300], 400, 2_300), { first: 2_280, last: 2_300, width: 1 },
    'a partial playhead day asks only that day through the playhead');
  assert.deepEqual(weatherTailWindow(branch, [2_400, 2_505], 400, 2_510), { first: 2_496, last: 2_510, width: 1 },
    'a zoom ending earlier within the current day still requests the current-day tail');
  assert.equal(weatherTailWindow(branch, [2_200, 2_495], 400, 2_510), null,
    'a zoom ending before the current day needs no current-day tail');
  assert.equal(weatherTailWindow(branch, [2_200, 2_303], 400, 2_303), null,
    'a completed day at 23:00 needs no partial-day tail');
  assert.deepEqual(weatherTailWindow(branch, [0, 50_003], 256, 50_003)?.last, 50_003,
    'folded views request only the final partial column through the playhead');
  assert.deepEqual(weatherWindow(branch, [0, 99_999], 256), { first: 0, last: 100_000, width: 256 }, 'whole days, within the branch');
  assert.deepEqual(shownRegions(reply, ['2']).map(region => region.name), ['Sea']);
  assert.equal(shownRegions(reply, []).length, 3);
  assert.deepEqual(uncoveredSpans([0, 100], { firstHour: 10, lastHour: 80 }), [{ firstHour: 0, lastHour: 9 }, { firstHour: 81, lastHour: 100 }]);
});

test('real loading order withholds future block readings while tail is pending, preserves history identity across hourly advance, and handles seek back and base change', async () => {
  const branch = { firstHour: 0, lastHour: 100_000 };
  const pending = new Map<string, { request: ProductRequest; resolve: (reply: WeatherReply) => void }>();
  const asked: { firstHour: number; lastHour: number; width: number }[] = [];
  const client = {
    product(request: ProductRequest) {
      const p = request.params as unknown as { firstHour: number; lastHour: number; width: number };
      asked.push({ firstHour: p.firstHour, lastHour: p.lastHour, width: p.width });
      const key = productKey(request);
      return new Promise(resolve => {
        pending.set(key, {
          request,
          resolve: data => resolve({ sourceId: request.sourceId, revision: request.revision, product: request.product, data }),
        });
      });
    },
  } as unknown as DashboardClient;

  const makeReq = (w: { first: number; last: number; width: number }) => ({
    sourceId: 's1', revision: 'r1', product: 'regional-weather',
    params: { branchId: '1', firstHour: w.first, lastHour: w.last, width: w.width, show: 'temperature' },
  }) as unknown as ProductRequest;

  const block2160 = validateWeather({
    ...reply, firstHour: 2160, lastHour: 4319, temperature: 'available',
    columns: {
      days: 1,
      columns: [
        { firstHour: 2472, lastHour: 2495, sampledHour: 2495, readHour: 2495, recorded: 1 },
        { firstHour: 2496, lastHour: 2519, sampledHour: 2519, readHour: 2519, recorded: 1 },
      ],
      regions: [{
        id: '1', observed: [1, 1], clear: [0, 0],
        measures: [
          { id: 'rain', days: [0, 1] },
          { id: 'temp', days: [1, 1], values: [['500000', '500000', '500000'], ['4500000', '4500000', '4500000']] },
        ],
      }],
    },
  });
  const tail2500 = validateWeather({
    ...reply, firstHour: 2496, lastHour: 2500, temperature: 'available',
    columns: {
      days: 1,
      columns: [{ firstHour: 2496, lastHour: 2500, sampledHour: 2500, readHour: 2500, recorded: 1 }],
      regions: [{
        id: '1', observed: [1], clear: [0],
        measures: [
          { id: 'snow', days: [1] },
          { id: 'temp', days: [1], values: [['600000', '600000', '600000']] },
        ],
      }],
    },
  });
  const tail2501 = validateWeather({
    ...reply, firstHour: 2496, lastHour: 2501, temperature: 'available',
    columns: {
      days: 1,
      columns: [{ firstHour: 2496, lastHour: 2501, sampledHour: 2501, readHour: 2501, recorded: 1 }],
      regions: [{
        id: '1', observed: [1], clear: [0],
        measures: [
          { id: 'snow', days: [1] },
          { id: 'temp', days: [1], values: [['700000', '700000', '700000']] },
        ],
      }],
    },
  });
  const block4320 = validateWeather({
    ...reply, firstHour: 4320, lastHour: 6479, temperature: 'available',
    columns: {
      days: 1,
      columns: [{ firstHour: 4320, lastHour: 4343, sampledHour: 4343, readHour: 4343, recorded: 1 }],
      regions: [{
        id: '1', observed: [1], clear: [0],
        measures: [{ id: 'temp', days: [1], values: [['1000000', '1000000', '1000000']] }],
      }],
    },
  });

  const scope = 's1:r1:1:temperature:null';
  let kept: WeatherFallback | null = null;
  const issue = (head: number, extent: [number, number] = [2160, head]) => {
    const baseWin = weatherWindow(branch, extent, 400);
    const tailWin = weatherTailWindow(branch, extent, 400, head);
    const baseReq = makeReq(baseWin);
    const tailReq = tailWin ? makeReq(tailWin) : null;
    const baseKey = productKey(baseReq);
    const tailKey = tailReq ? productKey(tailReq) : null;
    const basePromise = loadProduct(client, baseReq, new AbortController().signal, validateWeather);
    const tailPromise = tailReq ? loadProduct(client, tailReq, new AbortController().signal, validateWeather) : null;
    const snapshot = () => {
      const baseData = peekProduct(client, baseReq, baseKey, validateWeather);
      const tailData = tailReq && tailKey ? peekProduct(client, tailReq, tailKey, validateWeather) : undefined;
      const selected = selectWeatherReplies(client, scope, baseData, undefined, tailKey, tailData, undefined, kept);
      kept = selected.nextKept;
      const historyPeriods = selected.data ? weatherHistoryPeriods(selected.data.columns, extent, selected.data.dayHours, tailWin) : [];
      const tailPeriod = weatherTailPeriod(selected.tailData, extent);
      const loading = selected.data ? weatherLoadingSpans(extent, selected.data, tailWin, tailPeriod !== null) : [];
      const scale = selected.data ? weatherScale(selected.data, historyPeriods, 'temperature', null, extent, selected.tailData, tailPeriod) : null;
      return { ...selected, baseKey, tailKey, historyPeriods, tailPeriod, loading, scale };
    };
    return { baseKey, tailKey, basePromise, tailPromise, snapshot };
  };

  const resolvePending = async (key: string, data: WeatherReply) => {
    for (let i = 0; i < 5 && !pending.has(key); i++) await Promise.resolve();
    const entry = pending.get(key);
    assert.ok(entry, `expected pending request for ${key}`);
    pending.delete(key);
    entry.resolve(data);
  };

  const at2500 = issue(2500);
  await resolvePending(at2500.baseKey, block2160);
  await at2500.basePromise;
  const whileTail2500Pending = at2500.snapshot();
  assert.deepEqual(whileTail2500Pending.historyPeriods.map(p => p.firstHour), [2472], 'completed day stays available while tail loads');
  assert.equal(whileTail2500Pending.tailPeriod, null, 'pending current day is withheld');
  assert.deepEqual(whileTail2500Pending.loading, [{ firstHour: 2496, lastHour: 2500 }], 'pending current day is marked loading');
  assert.equal(whileTail2500Pending.scale?.high, 5, 'future 45C block reading is excluded from colour scale while tail loads');

  await resolvePending(at2500.tailKey!, tail2500);
  await at2500.tailPromise;
  const settled2500 = at2500.snapshot();
  assert.deepEqual(settled2500.loading, []);
  assert.equal(settled2500.tailPeriod?.readHour, 2500);
  assert.equal(settled2500.scale?.high, 6);
  const colsAt2500 = settled2500.data!.columns.columns;
  const daysAt2500 = settled2500.data!.columns.regions[0].measures[0].days;

  const at2501 = issue(2501);
  const whileTail2501Pending = at2501.snapshot();
  assert.equal(asked.filter(req => req.firstHour === 2160 && req.lastHour === 4319).length, 1, '90-day block is cached and requested once');
  assert.equal(whileTail2501Pending.tailData, undefined, 'old 2500 tail is never reused while 2501 tail is pending');
  assert.deepEqual(whileTail2501Pending.historyPeriods.map(p => p.firstHour), [2472], 'completed history stays available');
  assert.deepEqual(whileTail2501Pending.loading, [{ firstHour: 2496, lastHour: 2501 }], '2501 slot is marked loading');
  assert.equal(whileTail2501Pending.scale?.high, 5, 'future 45C block reading never leaks while advancing an hour');

  await resolvePending(at2501.tailKey!, tail2501);
  await at2501.tailPromise;
  const settled2501 = at2501.snapshot();
  assert.strictEqual(settled2501.data!.columns.columns, colsAt2500, 'historical columns object identity is preserved across hourly advance');
  assert.strictEqual(settled2501.data!.columns.regions[0].measures[0].days, daysAt2500, 'per-column measure arrays are never copied across hourly advance');
  assert.equal(settled2501.tailPeriod?.readHour, 2501);
  assert.equal(settled2501.scale?.high, 7);

  const seekBack2500 = issue(2500);
  const settledBack = seekBack2500.snapshot();
  assert.equal(settledBack.tailData, tail2500, 'seeking back retrieves exact 2500 tail, never 2501');

  const nextBlock = issue(4330, [4320, 4330]);
  const whileNextBlockPending = nextBlock.snapshot();
  assert.equal(whileNextBlockPending.tailData, undefined, 'when base changes, old tail from another window is never reused');
  assert.deepEqual(whileNextBlockPending.loading, [{ firstHour: 4320, lastHour: 4330 }], 'uncovered new block is marked loading');
  await resolvePending(nextBlock.baseKey, block4320);
  await nextBlock.basePromise;
  const afterNextBlockBase = nextBlock.snapshot();
  assert.equal(afterNextBlockBase.tailPeriod, null);
  assert.deepEqual(afterNextBlockBase.loading, [{ firstHour: 4320, lastHour: 4330 }]);
});

test('zoomed colour scale uses only the visible interval and region catalogue unions base and tail by region id', () => {
  const block = validateWeather({
    ...reply, firstHour: 0, lastHour: 2159, temperature: 'available',
    regions: [{ id: 'A', name: 'Alps' }, { id: 'B', name: 'Baltic' }],
    columns: {
      days: 1,
      columns: [
        { firstHour: 0, lastHour: 23, sampledHour: 23, readHour: 23, recorded: 1 },
        { firstHour: 1008, lastHour: 1031, sampledHour: 1031, readHour: 1031, recorded: 1 },
        { firstHour: 1032, lastHour: 1055, sampledHour: 1055, readHour: 1055, recorded: 1 },
      ],
      regions: [{
        id: 'A', observed: [1, 1, 1], clear: [0, 0, 0],
        measures: [
          { id: 'temp', days: [1, 1, 1], values: [['5000000', '5000000', '5000000'], ['500000', '500000', '500000'], ['1000000', '1000000', '1000000']] },
          { id: 'mod:air', days: [1, 1, 1], values: [['-90000', '-90000', '-90000'], ['-10000', '-10000', '-10000'], ['-20000', '-20000', '-20000']] },
        ],
      }],
    },
  });
  const periods = weatherPeriods(block.columns, [0, 2159], 24);
  const fullTempScale = weatherScale(block, periods, 'temperature', null, [0, 2159]);
  assert.equal(fullTempScale.high, 50);
  const zoomedTempScale = weatherScale(block, periods, 'temperature', null, [1000, 1167]);
  assert.equal(zoomedTempScale.high, 10, 'offscreen 50C extreme does not scale the zoomed 7-day window');
  const zoomedModScale = weatherScale(block, periods, 'modifiers', 'air', [1000, 1167]);
  assert.equal(zoomedModScale.highest, 0.2, 'offscreen modifier extreme does not scale the zoomed 7-day window');

  const tailWithNewRegion = validateWeather({
    ...reply, firstHour: 1056, lastHour: 1060, temperature: 'available',
    regions: [{ id: 'A', name: 'Alps' }, { id: 'C', name: 'Coast' }],
    columns: {
      days: 1,
      columns: [{ firstHour: 1056, lastHour: 1060, sampledHour: 1060, readHour: 1060, recorded: 1 }],
      regions: [{ id: 'C', observed: [1], clear: [0], measures: [{ id: 'rain', days: [1] }] }],
    },
  });
  const catalog = weatherCatalogRegions(block, tailWithNewRegion);
  assert.deepEqual(catalog.map(region => region.id), ['A', 'B', 'C'], 'catalogue unions base and tail by region id');
  const sameTail = { ...tailWithNewRegion, regions: [{ id: 'A', name: 'Alps' }, { id: 'B', name: 'Baltic' }] };
  assert.strictEqual(weatherCatalogRegions(block, sameTail), block.regions, 'catalogue keeps base array identity when tail adds no new regions');
  const tailPeriod = weatherTailPeriod(tailWithNewRegion, [0, 1060])!;
  const builtTail = weatherTail(tailWithNewRegion, catalog, tailPeriod, 'conditions', null, colours, { highest: 0, low: 0, high: 0 });
  assert.deepEqual(builtTail.cells[2].parts.map(part => part.id), ['rain'], 'tail-only region C receives its displayed cell');
});

test('malformed replies are refused: a read after its sample, a value without its days, a province of the wrong shape', () => {
  assert.throws(() => validateWeather({ ...reply, columns: { ...days, columns: [{ ...days.columns[0], readHour: 30 }, ...days.columns.slice(1)] } }));
  assert.throws(() => validateWeather({ ...reply, columns: { ...days, regions: [{ id: '1', measures: [{ id: 'mod:air', days: [0, 0, 0], values: [['1', '1', '1'], null, null] }] }] } }));
  assert.throws(() => validateWeather({ ...reply, columns: undefined }));
  assert.deepEqual(validateProvinces({ readHour: null, region: '1', provinces: [{ id: '5', name: null, snow: '100', mud: null, temperature: null }], warnings: [] }).provinces.length, 1);
  assert.throws(() => validateProvinces({ readHour: null, region: '1', provinces: [{ id: '5', name: null, snow: 1, mud: null, temperature: null }], warnings: [] }));
});
