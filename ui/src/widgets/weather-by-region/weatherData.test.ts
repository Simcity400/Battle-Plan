import test from 'node:test';
import assert from 'node:assert/strict';
import { columnTooltip, modifierText, shownRegions, temperatureText, uncoveredSpans, validateProvinces, validateWeather, weatherPeriods, weatherRows, weatherWindow } from './weatherData.ts';
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
  assert.deepEqual(weatherWindow(branch, [2_200, 2_300], 400, 100_000), { first: 2_160, last: 4_319, width: 90 });
  assert.deepEqual(weatherWindow(branch, [2_200, 2_300], 400, 2_300), { first: 2_160, last: 2_300, width: 6 },
    'snapping never pushes the asked window past the playhead ceiling');
  assert.deepEqual(weatherWindow(branch, [0, 50_003], 256, 50_003), { first: 0, last: 50_003, width: 256 },
    'folded windows stop at the playhead hour instead of the end of the day');
  assert.deepEqual(weatherWindow(branch, [0, 99_999], 256), { first: 0, last: 100_000, width: 256 }, 'whole days, within the branch');
  assert.deepEqual(shownRegions(reply, ['2']).map(region => region.name), ['Sea']);
  assert.equal(shownRegions(reply, []).length, 3);
  assert.deepEqual(uncoveredSpans([0, 100], { firstHour: 10, lastHour: 80 }), [{ firstHour: 0, lastHour: 9 }, { firstHour: 81, lastHour: 100 }]);
});

test('malformed replies are refused: a read after its sample, a value without its days, a province of the wrong shape', () => {
  assert.throws(() => validateWeather({ ...reply, columns: { ...days, columns: [{ ...days.columns[0], readHour: 30 }, ...days.columns.slice(1)] } }));
  assert.throws(() => validateWeather({ ...reply, columns: { ...days, regions: [{ id: '1', measures: [{ id: 'mod:air', days: [0, 0, 0], values: [['1', '1', '1'], null, null] }] }] } }));
  assert.throws(() => validateWeather({ ...reply, columns: undefined }));
  assert.deepEqual(validateProvinces({ readHour: null, region: '1', provinces: [{ id: '5', name: null, snow: '100', mud: null, temperature: null }], warnings: [] }).provinces.length, 1);
  assert.throws(() => validateProvinces({ readHour: null, region: '1', provinces: [{ id: '5', name: null, snow: 1, mud: null, temperature: null }], warnings: [] }));
});
