import test from 'node:test';
import assert from 'node:assert/strict';
import { keyedDays } from '../../dashboard/keyedDays.ts';
import { columnTooltip, modifierText, uncoveredSpans, shownRegions, validateProvinces, validateWeather, weatherPeriods, weatherRows, weatherTooltip, weatherWindow } from './weatherData.ts';
import type { WeatherColumns } from './weatherData';

const reply = validateWeather({
  window: { firstHour: 0, lastHour: 71, known: [{ firstHour: 0, lastHour: 47 }], reads: [[0, 23, 23], [24, 47, 47]], series: [
    { name: '1|rain', runs: [[0, 47, '1', true]] },
    { name: '1|snow', runs: [[24, 47, '1', true]] },
    { name: '1|mod:air', runs: [[0, 23, '-25000', true]] },
    { name: '2|rain', runs: [[0, 47, '0', true]] },
  ] },
  dayHours: 24, regions: [{ id: '1', name: 'Alps' }, { id: '2', name: 'Sea' }, { id: '3', name: 'Desert' }],
  flags: [{ id: 'rain', name: 'Rain' }, { id: 'snow', name: 'Snow' }], modifiers: [{ id: 'air', name: 'Air detection' }],
  theatres: [{ id: 't', name: 'Europe', regions: ['1'] }], temperature: 'Temperature unavailable', warnings: [],
});
const colours = { flag: (index: number) => `c${index}`, ramp: (value: number, highest: number) => `r${value / highest}`, clear: 'surface', gap: 'gap' };
/** The reply's three days, one per column, as the host folds them. */
const days: WeatherColumns = { days: 1, columns: [{ firstHour: 0, lastHour: 23, sampledHour: 23, readHour: 23, recorded: 1 },
  { firstHour: 24, lastHour: 47, sampledHour: 47, readHour: 47, recorded: 1 }, { firstHour: 48, lastHour: 71, sampledHour: null, readHour: null, recorded: 0 }], regions: [
  { id: '1', observed: [1, 1, 0], clear: [0, 0, 0], measures: [{ id: 'rain', days: [1, 1, 0] }, { id: 'snow', days: [0, 1, 0] },
    { id: 'mod:air', days: [1, 0, 0], values: [['-25000', '-25000', '-25000'], null, null] }] },
  { id: '2', observed: [1, 1, 0], clear: [1, 1, 0], measures: [] }] };
const folded = validateWeather({ ...reply, window: { ...reply.window, known: [], reads: [], series: [] }, columns: days });

test('overlapping flags all show as stripes and none set is blank', () => {
  const periods = weatherPeriods(days, [0, 71], 24);
  assert.deepEqual(periods.map(day => day.readHour), keyedDays(reply.window, [0, 71], 24).map(day => day.readHour), 'a column of one day reads as that day');
  const { rows } = weatherRows(folded, folded.regions, periods, 'conditions', null, colours);
  assert.deepEqual(rows[0].cells.map(cell => cell.parts.map(part => part.id)), [['rain'], ['rain', 'snow'], []]);
  assert.deepEqual(rows[0].cells[1].parts.map(part => part.share), [0.5, 0.5]);
  assert.deepEqual(rows[1].cells[0].parts, []);
  assert.equal(rows[2].cells[0].unknown, true);
});

test('a folded column sizes its stripes by days and keeps the clear days', () => {
  const week: WeatherColumns = { days: 7, columns: [{ firstHour: 0, lastHour: 167, sampledHour: 167, readHour: 160, recorded: 6 }],
    regions: [{ id: '1', observed: [6], clear: [3], measures: [{ id: 'rain', days: [2] }, { id: 'snow', days: [1] }] },
      { id: '2', observed: [6], clear: [6], measures: [] }, { id: '3', observed: [0], clear: [0], measures: [] }] };
  const data = validateWeather({ ...folded, columns: week });
  const { rows } = weatherRows(data, data.regions, weatherPeriods(week, [24, 100], 24), 'conditions', null, colours);
  // Six of seven days read: the unread day stands as not recorded, never clear.
  assert.deepEqual(rows[0].cells[0].parts.map(part => [part.id, part.share, part.color]),
    [['rain', 2 / 7, 'c0'], ['snow', 1 / 7, 'c1'], ['clear', 3 / 7, 'surface'], ['unread', 1 / 7, 'gap']]);
  assert.deepEqual(rows[1].cells[0].parts.map(part => [part.id, part.share]), [['clear', 6 / 7], ['unread', 1 / 7]]);
  const full = validateWeather({ ...folded, columns: { ...week, columns: [{ ...week.columns[0], recorded: 7 }], regions: [{ id: '2', observed: [7], clear: [7], measures: [] }] } });
  assert.deepEqual(weatherRows(full, full.regions, weatherPeriods(full.columns!, [0, 167], 24), 'conditions', null, colours).rows[1].cells[0].parts, [], 'no condition on any day is blank');
  assert.deepEqual(weatherPeriods(week, [24, 100], 24), [{ firstHour: 24, lastHour: 100, readHour: 160, column: 0, exact: false }], 'a folded column keeps the hour under the pointer');
  const unread = weatherRows(data, [...data.regions, { id: '3', name: 'Desert' }], weatherPeriods(week, [24, 100], 24), 'conditions', null, colours).rows;
  assert.equal(unread[2].cells[0].unknown, true, 'a region whose conditions were not read is unknown, never clear');
  assert.deepEqual(weatherPeriods({ ...week, columns: [{ ...week.columns[0], readHour: null }] }, [24, 100], 24)[0].readHour, 167, 'drawn at its sampled hour');
  assert.throws(() => validateWeather({ ...folded, columns: { ...week, regions: [{ id: '1', observed: [2], clear: [3], measures: [] }] } }), /malformed/u);
  assert.throws(() => validateWeather({ ...folded, columns: { ...week, regions: [{ id: '1', measures: [{ id: 'rain', days: [1, 2] }] }] } }), /malformed/u);
});

test('a modifier not witnessed is unknown, never zero', () => {
  const { rows, highest } = weatherRows(folded, folded.regions, weatherPeriods(days, [0, 47], 24), 'modifiers', 'air', colours);
  assert.equal(highest, 0.25);
  assert.equal(rows[0].cells[0].parts[0].color, 'r1');
  assert.equal(rows[0].cells[0].parts[0].label, '-0.25');
  assert.equal(rows[0].cells[1].unknown, true);
  assert.equal(modifierText('-25000'), '−25%');
});

test('theatres narrow regions and tooltips list the day', () => {
  assert.deepEqual(shownRegions(reply, [], ['t']).map(region => region.id), ['1']);
  const [, second] = keyedDays(reply.window, [0, 47], 24);
  assert.match(weatherTooltip(reply, reply.regions[0], second, 'Day 2', 'Day 2').rows.map(row => `${row.label}=${row.value}`).join(';'), /Conditions=Rain, Snow/);
  assert.equal(weatherTooltip(reply, reply.regions[0], null, 'Day 3', null).rows[1].label, 'Not recorded');
  for (const show of ['temperature', 'modifiers'] as const)
    assert.ok(!weatherTooltip(reply, reply.regions[0], second, 'Day 2', 'Day 2', show).rows.some(row => row.label === 'Conditions'), `a ${show} view lists no conditions`);
});

test('temperature shows each region\'s average, blank where it has none', () => {
  const withTemperature = validateWeather({ ...folded, columns: { ...days, regions: [{ id: '1', measures: [{ id: 'temp', days: [1, 0, 0], values: [['-250000', '-250000', '-250000'], null, null] }] }] } });
  const { rows } = weatherRows(withTemperature, withTemperature.regions, weatherPeriods(days, [0, 47], 24), 'temperature', null, { ...colours, temperature: (value, low, high) => `t${value}:${low}:${high}` });
  assert.equal(rows[0].cells[0].parts[0].color, 't-2.5:-2.5:0');
  assert.equal(rows[0].cells[0].parts[0].label, '−2.5 °C');
  assert.deepEqual(rows[0].cells[1].parts, []);
  assert.equal(rows[0].cells[1].unknown, undefined);
  const week = validateWeather({ ...folded, columns: { days: 2, columns: [{ firstHour: 0, lastHour: 47, sampledHour: 47, readHour: 47, recorded: 2 }],
    regions: [{ id: '1', measures: [{ id: 'temp', days: [2], values: [['-250000', '-100000', '150000']] }] }] } });
  const [mean] = weatherRows(week, week.regions, weatherPeriods(week.columns!, [0, 47], 24), 'temperature', null, { ...colours, temperature: (value, low, high) => `t${value}:${low}:${high}` }).rows[0].cells;
  assert.deepEqual(mean.parts.map(part => [part.label, part.color]), [['−0.5 °C', 't-0.5:-2.5:1.5']], 'a column shows its mean on the range of every day');
  assert.equal(mean.approximate, undefined);
  const partial = validateWeather({ ...week, columns: { ...week.columns!, regions: [{ id: '1', measures: [{ id: 'temp', days: [1], values: [['-250000', '-250000', '-250000']] }] }] } });
  assert.equal(weatherRows(partial, partial.regions, weatherPeriods(partial.columns!, [0, 47], 24), 'temperature', null, colours).rows[0].cells[0].approximate, true,
    'a value missing on some days marks the cell approximate');
});

test('a one-day column tooltip reads as that day; a folded one summarises its days', () => {
  const [first] = keyedDays(reply.window, [0, 23], 24);
  const day = weatherTooltip(reply, reply.regions[0], first, 'Day 1', 'Hour 23');
  assert.deepEqual(columnTooltip(folded, folded.regions[0], 0, 'conditions', null, 'Day 1', hour => `Hour ${hour}`), day);
  const week = validateWeather({ ...folded, columns: { days: 7, columns: [{ firstHour: 0, lastHour: 167, sampledHour: 167, readHour: 160, recorded: 6 }],
    regions: [{ id: '1', observed: [5], clear: [3], measures: [{ id: 'rain', days: [2] }, { id: 'temp', days: [4], values: [['-250000', '-200000', '50000']] }] }] } });
  const summary = columnTooltip(week, week.regions[0], 0, 'conditions', null, 'Day 2', hour => `Hour ${hour}`);
  assert.deepEqual(summary.rows.map(row => [row.label, row.value]), [['Hour 0 to Hour 167', undefined], ['Recorded days', '6 of 7'], ['Rain', '2 days'],
    ['No condition', '3 days'], ['Conditions not recorded', '2 days'], ['As of', 'Hour 160']]);
  const warmth = columnTooltip(week, week.regions[0], 0, 'temperature', null, 'Day 2', hour => `Hour ${hour}`);
  assert.ok(!columnTooltip(folded, folded.regions[0], 0, 'temperature', null, 'Day 1', hour => `Hour ${hour}`).rows.some(row => row.label === 'Conditions'),
    'a view without conditions lists none');
  assert.deepEqual(uncoveredSpans([0, 479], { firstHour: 0, lastHour: 239 }), [{ firstHour: 240, lastHour: 479 }]);
  assert.deepEqual(uncoveredSpans([100, 200], { firstHour: 0, lastHour: 479 }), []);
  assert.deepEqual(uncoveredSpans([100, 200], { firstHour: 300, lastHour: 479 }), [{ firstHour: 100, lastHour: 200 }]);
  assert.deepEqual(warmth.rows[2], { label: 'Temperature (average of provinces)', value: '−0.5 °C (−2.5 °C to 0.5 °C)' });
});

test('the provinces reply is tagged by its view and carries mud as a shown flag', () => {
  const reply = { view: 'provinces', hour: 10, readHour: 10, region: '71', warnings: [], provinces: [
    { id: '2', name: 'Overlay two', snow: '0', mud: false, temperature: '0' },
    { id: '3', name: null, snow: null, mud: null, temperature: null },
  ] };
  assert.deepEqual(validateProvinces(reply), reply);
  assert.throws(() => validateProvinces({ ...reply, view: 'history' }), /malformed/u);
  assert.throws(() => validateProvinces({ ...reply, provinces: [{ ...reply.provinces[0], mud: '17' }] }), /malformed/u);
});

test('a view up to the viewport keeps whole playback blocks of single days; a longer one folds its own days', () => {
  const branch = { firstHour: 0, lastHour: 2 * 365 * 24 - 1 };
  assert.deepEqual(weatherWindow(branch, [1000, 1167], 512), { first: 0, last: 90 * 24 - 1, width: 90 }, 'one column a day across the block');
  assert.deepEqual(weatherWindow(branch, [1000, 1167], 1024), weatherWindow(branch, [1000, 1167], 512), 'a resize asks nothing again where the days fit');
  // Blocks can hold more days than the view: never fewer columns than days then.
  assert.deepEqual(weatherWindow(branch, [100 * 24, 330 * 24], 256), { first: 90 * 24, last: 360 * 24 - 1, width: 270 });
  assert.deepEqual(weatherWindow(branch, [0, branch.lastHour], 256), { first: 0, last: branch.lastHour, width: 256 }, '730 days, three a column');
  assert.deepEqual(weatherWindow(branch, [30 * 24 + 5, 700 * 24 + 7], 256), { first: 30 * 24, last: 701 * 24 - 1, width: 256 }, 'whole days, from the first the view shows');
});
