import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import type { FamilyPlanContext, PointSourceRead } from '../../families/plan.ts';
import type { TimeChartProjection, TimeChartWidgetDeclaration } from '../../families/time-chart/declaration.ts';
import { deriveTimeChartTooltip, hitTestTimeChart } from '../../families/time-chart/hitTest.ts';
import { layoutTimeChart } from '../../families/time-chart/layout.ts';
import type { ResourceMonthsReply } from '../../generated/dashboard';
import { TIME_CHART_VIEWS } from '../manifest/time-chart.ts';

const FIRST = 60_000_000;
const MONTH = 720;
const HOUR = FIRST + 3 * MONTH + 100;

function context(): FamilyPlanContext {
  const branch = { id: '7', firstHour: FIRST, lastHour: FIRST + 5000 };
  return {
    track: { source: { id: 'src', revision: '1' }, axisOffset: 0, branch }, branch, lastRecordedHour: branch.lastHour, options: {}, following: true,
    aux: undefined, countryTag: 'GER', countryName: 'Germany', countries: { tags: ['GER'], membership: null },
    gateSpan: (firstHour: number, lastHour: number) => ({ firstHour, lastHour }),
  } as unknown as FamilyPlanContext;
}

const month = (index: number, net: number | null, missing: string | null = null) => ({
  firstHour: FIRST + index * MONTH, lastHour: Math.min(HOUR, FIRST + (index + 1) * MONTH - 1), net, recordedDays: net === null ? 0 : 30, days: 30, missing,
});

const REPLY: ResourceMonthsReply = {
  tag: 'GER', hour: HOUR, nextChange: HOUR + 20, warnings: [],
  resources: [
    { resource: { id: 1, token: 'steel', ordinal: 1, name: 'Steel', icon: null }, net: 144, missing: null, months: [month(0, 120), month(1, 130), month(2, null, 'Not read'), month(3, 140)] },
    { resource: { id: 2, token: 'rubber', ordinal: 1, name: null, icon: null }, net: null, missing: 'Resource balance was not recorded in this month.', months: [month(0, -20), month(1, -5), month(2, 3), month(3, 3.54)] },
  ],
};

test('the resource Time view reads one month reply at the hour and draws a row of signed month columns per resource', async () => {
  const entry = TIME_CHART_VIEWS.find(item => item.kind === 'resource-balance' && item.view === 'time');
  assert.ok(entry);
  const spec = await entry.loadSpec();
  const declaration = spec.declaration as TimeChartWidgetDeclaration<unknown>;
  const projection = spec.projection as TimeChartProjection<unknown>;
  const plan = declaration.sources(context(), { width: 800, height: 400 } as never);
  const read = plan.entries[0]!.read as PointSourceRead;
  assert.equal(read.product, 'resource-balance-months');
  assert.deepEqual(read.provenInterval?.(REPLY, HOUR), [HOUR, HOUR + 19]);

  const model = projection.project(createBoundedReadView({ hour: HOUR, panels: new Map([['months', REPLY]]) }), declaration.projectionOptions(context(), { extent: [FIRST, HOUR], surface: '#111111' }));
  assert.equal(model.mode, 'rows');
  assert.deepEqual(model.rows!.map(row => [row.name, row.valueText, row.valueMissing ?? null]), [['Steel', '+144', null], ['rubber', '', 'Resource balance was not recorded in this month.']]);
  assert.deepEqual(model.rows![1]!.months.map(m => m.valueText), ['−20', '−5', '+3', '+3.54']);
  assert.equal(model.rows![0]!.months[2]!.value, null);

  const layout = layoutTimeChart(model, 800, 300, 'Segoe UI', text => text.length * 6.5);
  const frame = layout.rows!;
  const steel = frame.rows.rows[0]!;
  const notRead = steel.row.months[2]!;
  const x = frame.toX((notRead.firstHour + notRead.lastHour) / 2);
  const hit = hitTestTimeChart(model, layout, [x, steel.zeroY]);
  assert.equal(hit, `cell:0:2@${notRead.firstHour}`);
  const tip = deriveTimeChartTooltip(model, layout, hit);
  assert.equal(tip?.payload?.title, 'Steel');
  assert.equal(tip?.payload?.note, 'Not read');
  const rubber = frame.rows.rows[1]!;
  const last = rubber.row.months[3]!;
  const lastTip = deriveTimeChartTooltip(model, layout, hitTestTimeChart(model, layout, [frame.toX(last.firstHour + 1), rubber.zeroY]));
  assert.equal(lastTip?.payload?.note, '+3.54 a day');
  assert.deepEqual(model.auditRows?.map(row => row.value), ['+144', 'Resource balance was not recorded in this month.']);

  const subset = projection.project(
    createBoundedReadView({ hour: HOUR, panels: new Map([['months', { ...REPLY, resources: [REPLY.resources[1]!] }]]) }),
    declaration.projectionOptions(context(), { extent: [FIRST, HOUR], surface: '#111111' }),
  );
  assert.equal(subset.rows![0]!.color, model.rows![1]!.color, 'filtering earlier resources preserves catalogue-id colour');
});
