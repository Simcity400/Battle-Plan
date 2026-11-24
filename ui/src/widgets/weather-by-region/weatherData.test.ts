import test from 'node:test';
import assert from 'node:assert/strict';
import { cellRect } from '../../charts/heat/layout.ts';
import type { DashboardClient, ProductRequest } from '../../dashboard/client';
import { loadProduct } from '../../dashboard/products.ts';
import { columnTooltip, mergeWeatherTail, modifierText, shownRegions, temperatureText, uncoveredSpans, validateProvinces, validateWeather, weatherPeriods, weatherRows, weatherTailWindow, weatherWindow } from './weatherData.ts';
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
  assert.equal(weatherTailWindow(branch, [2_200, 2_303], 400, 2_303), null,
    'a completed day at 23:00 needs no partial-day tail');
  assert.deepEqual(weatherTailWindow(branch, [0, 50_003], 256, 50_003)?.last, 50_003,
    'folded views request only the final partial column through the playhead');
  assert.deepEqual(weatherWindow(branch, [0, 99_999], 256), { first: 0, last: 100_000, width: 256 }, 'whole days, within the branch');
  assert.deepEqual(shownRegions(reply, ['2']).map(region => region.name), ['Sea']);
  assert.equal(shownRegions(reply, []).length, 3);
  assert.deepEqual(uncoveredSpans([0, 100], { firstHour: 10, lastHour: 80 }), [{ firstHour: 0, lastHour: 9 }, { firstHour: 81, lastHour: 100 }]);
});

test('consecutive follow playheads in one 90-day block reuse the cached block request and only fetch the current partial day', async () => {
  const branch = { firstHour: 0, lastHour: 100_000 };
  const asked: { firstHour: number; lastHour: number; width: number }[] = [];
  const client = {
    async product(request: ProductRequest) {
      const p = request.params as unknown as { firstHour: number; lastHour: number; width: number };
      asked.push({ firstHour: p.firstHour, lastHour: p.lastHour, width: p.width });
      return {
        sourceId: request.sourceId, revision: request.revision, product: request.product,
        data: {
          ...reply, firstHour: p.firstHour, lastHour: p.lastHour,
          columns: { days: 1, columns: [{ firstHour: p.firstHour, lastHour: p.lastHour, sampledHour: p.lastHour, readHour: p.lastHour, recorded: 1 }],
            regions: [{ id: '1', observed: [1], clear: [0], measures: [{ id: 'rain', days: [1] }] }] },
        },
      };
    },
  } as unknown as DashboardClient;
  const runFor = async (head: number) => {
    const extent: [number, number] = [head - 168, head];
    const base = weatherWindow(branch, extent, 400);
    const tail = weatherTailWindow(branch, extent, 400, head);
    const req = (w: { first: number; last: number; width: number }) => ({
      sourceId: 's1', revision: 'r1', product: 'regional-weather',
      params: { branchId: '1', firstHour: w.first, lastHour: w.last, width: w.width, show: 'conditions' },
    }) as unknown as ProductRequest;
    await loadProduct(client, req(base), new AbortController().signal, validateWeather);
    if (tail) await loadProduct(client, req(tail), new AbortController().signal, validateWeather);
  };
  await runFor(2500);
  await runFor(2501);
  const blockRequests = asked.filter(item => item.firstHour === 2160 && item.lastHour === 4319);
  const tailRequests = asked.filter(item => item.width === 1);
  assert.equal(blockRequests.length, 1, '90-day block is requested once and reused across advancing playheads');
  assert.deepEqual(tailRequests, [
    { firstHour: 2496, lastHour: 2500, width: 1 },
    { firstHour: 2496, lastHour: 2501, width: 1 },
  ], 'only the current partial day is requested on each advancing hour');
});

test('midnight at hour 2160 replaces the block day with the midnight reading and gives the cell drawable width', () => {
  const block = validateWeather({
    ...reply, firstHour: 2160, lastHour: 4319,
    columns: {
      days: 1,
      columns: [
        { firstHour: 2160, lastHour: 2183, sampledHour: 2183, readHour: 2183, recorded: 1 },
        { firstHour: 2184, lastHour: 2207, sampledHour: 2207, readHour: 2207, recorded: 1 },
      ],
      regions: [{ id: '1', observed: [1, 1], clear: [0, 0], measures: [{ id: 'rain', days: [1, 1] }] }],
    },
  });
  const midnightTail = validateWeather({
    ...reply, firstHour: 2160, lastHour: 2160,
    columns: {
      days: 1,
      columns: [{ firstHour: 2160, lastHour: 2160, sampledHour: 2160, readHour: 2160, recorded: 1 }],
      regions: [{ id: '1', observed: [1], clear: [0], measures: [{ id: 'snow', days: [1] }] }],
    },
  });
  const merged = mergeWeatherTail(block, midnightTail);
  const periods = weatherPeriods(merged.columns, [1992, 2160], 24);
  assert.equal(periods.length, 1);
  assert.equal(periods[0].firstHour, 2160);
  assert.equal(periods[0].lastHour, 2160);
  assert.equal(periods[0].readHour, 2160);
  const { rows } = weatherRows(merged, [reply.regions[0]], periods, 'conditions', null, colours);
  assert.deepEqual(rows[0].cells[0].parts.map(part => part.id), ['snow'], 'midnight reading replaces end-of-day rain');
  const frame = { plotTop: 20, plotHeight: 100, plotLeft: 80, plotWidth: 336, toX: (h: number) => 80 + (h - 1992) * 2 };
  const rect = cellRect(periods[0], 0, 1, frame, 1992, 2160, 2160);
  assert.ok(rect && rect.width >= 1, 'cell starting at the terminal midnight hour has drawable width');
});

test('malformed replies are refused: a read after its sample, a value without its days, a province of the wrong shape', () => {
  assert.throws(() => validateWeather({ ...reply, columns: { ...days, columns: [{ ...days.columns[0], readHour: 30 }, ...days.columns.slice(1)] } }));
  assert.throws(() => validateWeather({ ...reply, columns: { ...days, regions: [{ id: '1', measures: [{ id: 'mod:air', days: [0, 0, 0], values: [['1', '1', '1'], null, null] }] }] } }));
  assert.throws(() => validateWeather({ ...reply, columns: undefined }));
  assert.deepEqual(validateProvinces({ readHour: null, region: '1', provinces: [{ id: '5', name: null, snow: '100', mud: null, temperature: null }], warnings: [] }).provinces.length, 1);
  assert.throws(() => validateProvinces({ readHour: null, region: '1', provinces: [{ id: '5', name: null, snow: 1, mud: null, temperature: null }], warnings: [] }));
});
