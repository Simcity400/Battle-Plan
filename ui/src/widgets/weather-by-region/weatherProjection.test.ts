import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { projectLaneRunsToPixels } from '../../families/heat/lanesLayout.ts';
import { deriveLanesTooltip } from '../../families/heat/lanesModel.ts';
import type { RegionalWeatherReply } from '../../generated/dashboard';
import { projectWeather } from './weatherProjection.ts';

const SURFACE = '#161616';

function reply(lastHour: number, extra: Partial<RegionalWeatherReply> = {}): RegionalWeatherReply {
  return {
    firstHour: 0,
    lastHour,
    dayHours: 24,
    regions: [{ id: '5', name: 'Southern England' }, { id: '6', name: 'Scotland' }],
    flags: [{ id: 'light_rain', name: 'Light rain' }, { id: 'heavy_rain', name: 'Heavy rain' }],
    modifiers: [],
    temperature: 'available',
    columns: {
      days: 1,
      columns: [
        { firstHour: 0, lastHour: 23, sampledHour: 23, readHour: 23, recorded: 1 },
        { firstHour: 24, lastHour: 47, sampledHour: null, readHour: null, recorded: 0 },
        { firstHour: 48, lastHour: lastHour, sampledHour: lastHour, readHour: lastHour, recorded: 1 },
      ],
      regions: [
        {
          id: '5', pinned: true, observed: [1, 0, 1], clear: [0, 0, 0],
          measures: [{ id: 'light_rain', days: [1, 0, 1] }],
          readout: { kind: 'condition', id: 'light_rain', name: 'Light rain', since_hour: 60780000, since_bounded: true, as_of_hour: lastHour },
        },
        {
          id: '6', pinned: false, observed: [1, 0, 1], clear: [1, 0, 1], measures: [],
          readout: { kind: 'condition', id: 'clear', name: 'Clear', since_hour: 48, since_bounded: false, as_of_hour: lastHour },
        },
      ],
    },
    legend: [],
    gaps: [],
    warnings: [],
    ...extra,
  };
}

function view(hour: number, data: RegionalWeatherReply) {
  return createBoundedReadView({ hour, panels: new Map([['weather', data]]) });
}

const OPTIONS = { show: 'conditions' as const, modifier: null, surface: SURFACE };

test('the current day is drawn once, with width, at midnight', () => {
  const model = projectWeather(view(48, reply(48)), OPTIONS);
  const runs = model.rows[0]!.runs;
  const last = runs.at(-1)!;
  assert.equal(last.firstHour, 48);
  assert.equal(last.lastHour, 48);
  assert.equal(runs.filter(run => run.firstHour <= 48 && 48 <= run.lastHour).length, 1);
  const spans = projectLaneRunsToPixels(runs, model.firstHour, model.lastHour, model.hour, 0, 400);
  assert.ok(spans.at(-1)!.width > 0, 'the terminal day has drawable width');
  assert.equal(model.playhead, 48);
});

test('a reply that reaches past the displayed hour draws nothing from it', () => {
  const model = projectWeather(view(50, reply(60)), OPTIONS);
  assert.deepEqual(model.rows, []);
});

test('an unrecorded day is the hatch with its reason, never clear', () => {
  const model = projectWeather(view(48, reply(48)), OPTIONS);
  const gap = model.rows[0]!.runs.find(run => run.firstHour === 24)!;
  assert.deepEqual(gap.cell, { kind: 'missing', reason: 'Weather was not recorded on these days' });
  assert.equal(model.rows[1]!.runs[0]!.cell.kind, 'clear');
});

test('a condition reads since the day it began, or observed since its first sampled day, and a clear region as Clear', () => {
  const model = projectWeather(view(48, reply(48)), OPTIONS);
  const rain = model.rows[0]!.readout!;
  assert.equal(rain.kind, 'value');
  assert.equal(rain.kind === 'value' && rain.form, 'word');
  assert.equal(rain.kind === 'value' && rain.primary, 'Light rain');
  assert.match(rain.kind === 'value' ? rain.secondary ?? '' : '', /^Observed since \d+ \w{3}$/, 'the first sampled day is a bound, not the start');
  assert.match(rain.kind === 'value' ? rain.audit ?? '' : '', /^Light rain observed since \d+ \w{3}$/);
  const seen = reply(48);
  const exact = { ...seen, columns: { ...seen.columns, regions: seen.columns.regions.map((region, index) => (index === 0 && region.readout?.kind === 'condition' ? { ...region, readout: { ...region.readout, since_bounded: false } } : region)) } };
  const started = projectWeather(view(48, exact), OPTIONS).rows[0]!.readout!;
  assert.match(started.kind === 'value' ? started.secondary ?? '' : '', /^Since \d+ \w{3}$/, 'a start the recording saw is exact');
  const clear = model.rows[1]!.readout!;
  assert.equal(clear.kind === 'value' && clear.swatch, 'hollow');
  assert.equal(model.rows[0]!.pinned, true);
  assert.equal(model.rows[1]!.pinned, false);
});

test('a temperature column shows the mean of its days and a missing reading names why', () => {
  const data = reply(48, {
    columns: {
      days: 1,
      columns: [
        { firstHour: 0, lastHour: 23, sampledHour: 23, readHour: 23, recorded: 1 },
        { firstHour: 24, lastHour: 48, sampledHour: 48, readHour: 48, recorded: 1 },
      ],
      regions: [{
        id: '5', pinned: false,
        measures: [{ id: 'temp', days: [1, 0], values: [['1250000', '1250000', '1250000'], null] }],
        readout: { kind: 'notRecorded', reason: 'Weather reading was not recorded at this hour' },
      }],
    },
    extrema: ['1250000', '1250000'],
  });
  const model = projectWeather(view(48, data), { show: 'temperature', modifier: null, surface: SURFACE });
  const [first, second] = model.rows[0]!.runs;
  assert.equal(deriveLanesTooltip(model, 0, first!.firstHour)!.items![0]!.value, '12.5 °C');
  assert.deepEqual(second!.cell, { kind: 'missing', reason: 'Weather reading was not recorded' });
  assert.deepEqual(model.rows[0]!.readout, { kind: 'missing', reason: 'Weather reading was not recorded at this hour' });

  const smallNeg = reply(48, {
    columns: {
      days: 1,
      columns: [{ firstHour: 0, lastHour: 48, sampledHour: 48, readHour: 48, recorded: 1 }],
      regions: [{
        id: '5', pinned: false,
        measures: [{ id: 'temp', days: [1], values: [['-4000', '-4000', '-4000']] }],
        readout: { kind: 'value', value: '255000', as_of_hour: 48 },
      }],
    },
    extrema: ['-4000', '255000'],
  });
  const smallModel = projectWeather(view(48, smallNeg), { show: 'temperature', modifier: null, surface: SURFACE });
  assert.equal(deriveLanesTooltip(smallModel, 0, 0)!.items![0]!.value, '−<0.1 °C');
  assert.equal(smallModel.rows[0]!.readout?.kind === 'value' ? smallModel.rows[0]!.readout.primary : '', '2.5 °C');
});

test('a folded column with unrecorded days keeps them unknown instead of spreading the known days over the column', () => {
  const data = reply(95, {
    columns: {
      days: 4,
      columns: [{ firstHour: 0, lastHour: 95, sampledHour: 95, readHour: 95, recorded: 2 }],
      regions: [{ id: '5', pinned: false, observed: [2], clear: [1], measures: [{ id: 'light_rain', days: [1] }] }],
    },
  });
  const cell = projectWeather(view(95, data), OPTIONS).rows[0]!.runs[0]!.cell;
  assert.equal(cell.kind, 'parts');
  const shares = cell.kind === 'parts' ? cell.parts.map(part => [part.id, part.share]) : [];
  assert.deepEqual(shares, [['light_rain', 0.25], ['clear', 0.25]]);
});

test('conditions take their colour from the identity register, and an unknown one from the palette, never a registered colour', async () => {
  const { identityColour, readableColour, IDENTITY_COLOURS } = await import('../../charts/palette.ts');
  assert.equal(identityColour(SURFACE, 'weather', 'heavy_rain', 0), readableColour(IDENTITY_COLOURS.weather.heavy_rain, SURFACE), 'heavy rain is the storm violet whatever its order');
  assert.equal(identityColour(SURFACE, 'weather', 'light_rain', 5), readableColour(IDENTITY_COLOURS.weather.light_rain, SURFACE));
  const registered = new Set(Object.values(IDENTITY_COLOURS.weather).map(hex => readableColour(hex, SURFACE)));
  for (let ordinal = 0; ordinal < 12; ordinal++) assert.ok(!registered.has(identityColour(SURFACE, 'weather', 'mod_acid_rain', ordinal)), 'a mod condition gets its own colour');
});
