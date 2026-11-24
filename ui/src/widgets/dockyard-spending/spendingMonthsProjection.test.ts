import assert from 'node:assert/strict';
import test from 'node:test';
import { firstHourOfMonth } from '../../dashboard/calendar.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { deriveTimeChartTooltip } from '../../families/time-chart/hitTest.ts';
import { layoutTimeChart } from '../../families/time-chart/layout.ts';
import type { ProductionSpendingMonthsReply } from '../../generated/dashboard';
import { factoryMonthsProjection } from '../military-factory-spending/factoryMonthsProjection.ts';
import { dockyardMonthsProjection } from './dockyardMonthsProjection.ts';
import { formatExactIcText } from './spendingMonthsProjection.ts';

const measure = (text: string) => text.length * 6.5;

const jan1 = firstHourOfMonth(1936 * 12);
const feb1 = firstHourOfMonth(1936 * 12 + 1);
const mar1 = firstHourOfMonth(1936 * 12 + 2);
const apr1 = firstHourOfMonth(1936 * 12 + 3);

function dockyardReply(): ProductionSpendingMonthsReply {
  return {
    branchId: '1',
    tag: 'GER',
    domain: 'naval',
    decimals: 5,
    recorded: true,
    bands: [
      { id: 'new', label: 'New ships', remainder: false, denominator: 1 },
      { id: 'refit', label: 'Refits', remainder: false, denominator: 1 },
      { id: 'repair', label: 'Repairs', remainder: false, denominator: 24 },
      { id: 'purpose-unrecorded', label: 'Purpose not recorded', remainder: true, denominator: 1 },
      { id: 'other-purpose', label: 'Other purposes', remainder: true, denominator: 1 },
    ],
    months: [
      {
        year: 1936,
        month: 1,
        firstHour: jan1,
        lastHour: feb1 - 1,
        monthFirstHour: jan1,
        monthLastHour: feb1 - 1,
        incomplete: false,
        parts: [
          { raw: '3000000', value: 30, reason: null, uncertain: false },
          { raw: '500000', value: 5, reason: null, uncertain: false },
          { raw: '14400000', value: 6, reason: null, uncertain: false },
          { raw: '0', value: 0, reason: null, uncertain: false },
          { raw: '0', value: 0, reason: null, uncertain: false },
        ],
        totalRaw: '98400000',
        totalDenominator: 24,
        totalValue: 41,
        totalReason: null,
        note: null,
      },
      {
        year: 1936,
        month: 2,
        firstHour: feb1,
        lastHour: mar1 - 1,
        monthFirstHour: feb1,
        monthLastHour: mar1 - 1,
        incomplete: false,
        parts: [
          { raw: null, value: null, reason: 'Spending unavailable at this time.', uncertain: false },
          { raw: null, value: null, reason: 'Spending unavailable at this time.', uncertain: false },
          { raw: '24000000', value: 10, reason: null, uncertain: false },
          { raw: '1000000', value: 10, reason: null, uncertain: false },
          { raw: '0', value: 0, reason: null, uncertain: false },
        ],
        totalRaw: null,
        totalDenominator: 24,
        totalValue: null,
        totalReason: 'Spending unavailable at this time.',
        note: 'Whether some of it was refits was not recorded.',
      },
      {
        year: 1936,
        month: 3,
        firstHour: mar1,
        lastHour: mar1 + 10 * 24,
        monthFirstHour: mar1,
        monthLastHour: apr1 - 1,
        incomplete: true,
        parts: [
          { raw: null, value: null, reason: 'Spending not recorded.', uncertain: false },
          { raw: null, value: null, reason: 'Spending not recorded.', uncertain: false },
          { raw: null, value: null, reason: 'Spending not recorded.', uncertain: false },
          { raw: null, value: null, reason: 'Spending not recorded.', uncertain: false },
          { raw: null, value: null, reason: 'Spending not recorded.', uncertain: false },
        ],
        totalRaw: null,
        totalDenominator: 24,
        totalValue: null,
        totalReason: 'Spending not recorded.',
        note: null,
      },
    ],
    nextChangeHour: null,
    warnings: [],
  };
}

test('dockyard spending by month plots stacked columns, reserved grey unknown-purpose band, missing-part strips, and exact tooltips', () => {
  const reply = dockyardReply();
  const hour = mar1 + 10 * 24;
  const view = createBoundedReadView({ hour, panels: new Map([['main', reply]]) });
  const model = dockyardMonthsProjection.project(view, {
    domain: 'naval',
    measureTitle: 'Dockyard IC per month',
    digits: 1,
    surface: '#111111',
    extent: [jan1, apr1 - 1],
  });

  assert.equal(model.mode, 'columns');
  assert.equal(model.columns?.foldUnrecorded, false);
  assert.equal(model.columns?.months.length, 3);
  assert.deepEqual(
    model.legend?.map(entry => entry.id),
    ['new', 'refit', 'repair', 'purpose-unrecorded', '__unrecorded__'],
  );

  const jan = model.columns!.months[0]!;
  assert.equal(jan.status, 'recorded');
  assert.equal(jan.totalText, '41 IC');
  assert.equal(jan.missingParts, undefined);

  const feb = model.columns!.months[1]!;
  assert.equal(feb.status, 'recorded');
  assert.equal(feb.segments.length, 2);
  assert.equal(feb.missingParts?.length, 2);
  assert.equal(feb.totalMissingReason, 'Spending unavailable at this time.');

  const mar = model.columns!.months[2]!;
  assert.equal(mar.status, 'not-recorded');
  assert.equal(mar.reason, 'Spending not recorded.');

  const layout = layoutTimeChart(model, 720, 260, 'sans-serif', measure);
  const tip = deriveTimeChartTooltip(model, layout, `month:1@${feb1}`);
  assert.ok(tip?.payload);
  assert.equal(tip.payload.items?.some(item => item.label === 'New ships' && item.missingReason === 'Spending unavailable at this time.'), true);
  assert.equal(tip.payload.items?.some(item => item.label === 'Total' && item.missingReason === 'Spending unavailable at this time.'), true);
  assert.equal(tip.payload.note, 'Whether some of it was refits was not recorded.');
});

test('military factory spending by month handles unrecorded and known-zero campaigns honestly', () => {
  assert.equal(formatExactIcText('14640000', 24, 5, 1), '6.1 IC');

  const unrecorded: ProductionSpendingMonthsReply = {
    branchId: '1',
    tag: 'GER',
    domain: 'air',
    decimals: 5,
    recorded: false,
    bands: [
      { id: 'new', label: 'New', remainder: false, denominator: 1 },
      { id: 'converted', label: 'Converted', remainder: false, denominator: 1 },
      { id: 'purpose-unrecorded', label: 'Purpose not recorded', remainder: true, denominator: 1 },
      { id: 'other-purpose', label: 'Other purposes', remainder: true, denominator: 1 },
    ],
    months: [],
    nextChangeHour: null,
    warnings: [],
  };
  const unrecModel = factoryMonthsProjection.project(
    createBoundedReadView({ hour: feb1, panels: new Map([['main', unrecorded]]) }),
    { domain: 'air', measureTitle: 'Air factory IC per month', digits: 0, surface: '#111111', extent: [jan1, feb1] },
  );
  assert.equal(unrecModel.empty?.kind, 'unavailable');

  const knownZero: ProductionSpendingMonthsReply = {
    ...unrecorded,
    recorded: true,
    months: [{
      year: 1936,
      month: 1,
      firstHour: jan1,
      lastHour: feb1 - 1,
      monthFirstHour: jan1,
      monthLastHour: feb1 - 1,
      incomplete: false,
      parts: [
        { raw: '0', value: 0, reason: null, uncertain: false },
        { raw: '0', value: 0, reason: null, uncertain: false },
        { raw: '0', value: 0, reason: null, uncertain: false },
        { raw: '0', value: 0, reason: null, uncertain: false },
      ],
      totalRaw: '0',
      totalDenominator: 1,
      totalValue: 0,
      totalReason: null,
      note: null,
    }],
  };
  const zeroModel = factoryMonthsProjection.project(
    createBoundedReadView({ hour: feb1, panels: new Map([['main', knownZero]]) }),
    { domain: 'air', measureTitle: 'Air factory IC per month', digits: 0, surface: '#111111', extent: [jan1, feb1] },
  );
  assert.equal(zeroModel.empty?.kind, 'empty');
  assert.equal(zeroModel.empty?.message, 'No military factory IC spent at this date.');
});
