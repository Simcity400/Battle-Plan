import test from 'node:test';
import assert from 'node:assert/strict';
import { keyedDays } from '../../dashboard/keyedDays.ts';
import { modifierText, shownRegions, validateProvinces, validateWeather, weatherRows, weatherTooltip } from './weatherData.ts';

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
const colours = { flag: (index: number) => `c${index}`, ramp: (value: number, highest: number) => `r${value / highest}` };

test('overlapping flags all show as stripes and none set is blank', () => {
  const days = keyedDays(reply.window, [0, 71], 24);
  assert.deepEqual(days.map(day => day.readHour), [23, 47, null]);
  const { rows } = weatherRows(reply, reply.regions, days, 'conditions', null, colours);
  assert.deepEqual(rows[0].cells.map(cell => cell.parts.map(part => part.id)), [['rain'], ['rain', 'snow'], []]);
  assert.deepEqual(rows[0].cells[1].parts.map(part => part.share), [0.5, 0.5]);
  assert.deepEqual(rows[1].cells[0].parts, []);
  assert.equal(rows[2].cells[0].unknown, true);
});

test('a modifier not witnessed is unknown, never zero', () => {
  const days = keyedDays(reply.window, [0, 47], 24);
  const { rows, highest } = weatherRows(reply, reply.regions, days, 'modifiers', 'air', colours);
  assert.equal(highest, 0.25);
  assert.equal(rows[0].cells[0].parts[0].color, 'r1');
  assert.equal(rows[0].cells[1].unknown, true);
  assert.equal(modifierText('-25000'), '−25%');
});

test('theatres narrow regions and tooltips list the day', () => {
  assert.deepEqual(shownRegions(reply, [], ['t']).map(region => region.id), ['1']);
  const [, second] = keyedDays(reply.window, [0, 47], 24);
  assert.match(weatherTooltip(reply, reply.regions[0], second, 'Day 2', 'Day 2').rows.map(row => `${row.label}=${row.value}`).join(';'), /Conditions=Rain, Snow/);
  assert.equal(weatherTooltip(reply, reply.regions[0], null, 'Day 3', null).rows[1].label, 'Not recorded');
});

test('temperature shows each region\'s average, blank where it has none', () => {
  const withTemperature = validateWeather({ ...reply, window: { ...reply.window, series: [...reply.window.series, { name: '1|temp', runs: [[0, 23, '-250000', true]] }] } });
  const days = keyedDays(withTemperature.window, [0, 47], 24);
  const { rows } = weatherRows(withTemperature, withTemperature.regions, days, 'temperature', null, { ...colours, temperature: (value, low, high) => `t${value}:${low}:${high}` });
  assert.equal(rows[0].cells[0].parts[0].color, 't-2.5:-2.5:0');
  assert.equal(rows[0].cells[0].parts[0].label, '−2.5 °C');
  assert.deepEqual(rows[0].cells[1].parts, []);
  assert.equal(rows[0].cells[1].unknown, undefined);
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
