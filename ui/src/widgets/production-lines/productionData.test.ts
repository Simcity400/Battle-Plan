import assert from 'node:assert/strict';
import test from 'node:test';
import { liveKey } from '../../charts/live.ts';
import { valuesOf } from '../../test-support/liveValues.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { compareValues, knownValue } from '../../families/value.ts';
import { registerWidgets, widgetUnavailable } from '../../dashboard/definitions.ts';
import { createWidget } from '../../dashboard/model.ts';
import { definition } from './definition.ts';
import { efficiencyPercent, iconImage, validateAtReply, validateBreakdownReply, variantName, variantOf, weeklyRaw } from './productionData.ts';
import { productionLinesProjection, type ProductionLineMeta } from './productionLinesProjection.ts';
import type { LineNow, ProductionLineBreakdownReply, ProductionLinesAtReply } from '../../generated/dashboard';

const now = (handle: string, priority: number): LineNow => ({ handle, kind: 'military', variant: '100', mio: '0', maxFactories: 10, priority,
  output: { parts: [{ variant: '100', mio: '0', exact: true }], sinceHour: 1000, wholeLife: true } });

const at = (lines: LineNow[]): ProductionLinesAtReply => ({ branchId: '1', tag: 'GER', decimals: 5, missing: null, outputMissing: null, organisations: [], icons: [], warnings: [],
  variants: [{ id: '100', hour: 1300, known: true, token: 'infantry_equipment', name: 'Rifle', typeName: 'Infantry Equipment', nameUnknown: null, icon: null, iconUnknown: null }], lines });

const project = (reply: ProductionLinesAtReply, values: Map<string, number>, country: string | null = 'GER') =>
  productionLinesProjection.project(createBoundedReadView({ hour: 1200, panels: new Map([['main', reply]]), live: new Map([['main', valuesOf(values, 'not recorded')]]) }), { country });

test('a running line paints its numbers from its cells, exact as the game computes them, and a cell without a value is missing, never zero', () => {
  const reply = at([now('9', 0), now('11', 1)]);
  assert.deepEqual(validateAtReply(reply), reply);
  assert.throws(() => validateAtReply({ ...reply, lines: [{ ...reply.lines[0], handle: 'x' }] }), /Invalid production lines/);
  const model = project(reply, new Map([[liveKey('9', 'active'), 4], [liveKey('9', 'efficiency'), 1_638_400], [liveKey('9', 'speed'), 200_000], [liveKey('9', 'cost'), 700_000]]));
  const [running, unread] = model.rows;
  assert.deepEqual(running!.cells.efficiency!.value, { kind: 'known', value: '50%' });
  assert.equal(efficiencyPercent(1_638_400n), 50n);
  assert.equal(weeklyRaw(200_000n, 700_000n), 200_000n, 'speed × 700000 ÷ cost, truncated');
  assert.equal(weeklyRaw(1n, 3n), 233_333n);
  assert.equal(weeklyRaw(1n, 0n), null);
  assert.equal(unread!.cells.factories!.value.kind, 'notRecorded');
  assert.equal(variantName(variantOf(reply.variants, '100'), '100'), 'Rifle');
});

test('a line shows its recorded maximum factories, and output counted since a later start is the figure marked partial', () => {
  const reply = at([{ ...now('9', 0), maxFactories: 15, output: { parts: [{ variant: '100', mio: '0', exact: true }], sinceHour: 1000, wholeLife: false } }]);
  const cells = project(reply, new Map([[liveKey('9', 'active'), 4], [liveKey('9', 'requested'), 6], [liveKey('9', 'output'), 12]])).rows[0]!.cells;
  assert.deepEqual([cells.factories!.value, cells.factories!.secondaryValue], [{ kind: 'known', value: 4 }, { kind: 'known', value: 15 }], 'assigned / the recorded maximum, never the requested count');
  assert.equal(cells.total!.value.kind, 'partial');
  assert.equal(cells.total!.value.kind === 'partial' && cells.total!.value.value, 12);
  const sorted = cells.total!.sortValue ?? cells.total!.value;
  for (const direction of ['ascending', 'descending'] as const) {
    assert.ok(compareValues(sorted, knownValue(10), direction) > 0, `a partial lifetime total is unranked, after complete ones, ${direction}`);
  }
});

test('a row carries its line and country, so a freshly drawn table opens the breakdown at the clicked hour without any history', () => {
  const row = project(at([now('9', 0)]), new Map()).rows[0]!;
  assert.deepEqual(row.meta, { handle: '9', country: 'GER', name: 'Rifle' } satisfies ProductionLineMeta);
  const group = (id: string, totalRaw: string) => ({ id, label: id, totalRaw, decimals: 5, unit: '%', parts: [{ id: 'base', label: 'Base', raw: totalRaw, unknown: null }], note: '' });
  const breakdown: ProductionLineBreakdownReply = { branchId: '1', tag: 'GER', hour: 1200, groups: [{ id: 'efficiency', label: 'Efficiency' }], group: group('efficiency', '2000'),
    missing: null, warnings: ['Organisation bonuses were pending refresh at this read; retained values are shown'] };
  assert.deepEqual(validateBreakdownReply(breakdown), breakdown);
  const unread = { ...breakdown, group: null, missing: "The country's production modifiers were not recorded at this hour" };
  assert.deepEqual(validateBreakdownReply(unread), unread);
  assert.throws(() => validateBreakdownReply({ ...breakdown, group: null }), 'a factor with no reading carries the reason it is missing');
});

test('a picture is drawn only from an artwork URL; an image no host serves is missing art, never a broken link', () => {
  const served = `http://artwork.localhost/${'0123456789abcdef'.repeat(2)}.png`;
  const icons = [{ key: 'sprite:GFX_a#1', image: served }, { key: 'sprite:GFX_b#2', image: `artwork:${'0123456789abcdef'.repeat(2)}.png` }];
  assert.equal(iconImage(icons, 'sprite:GFX_a#1'), served);
  assert.equal(iconImage(icons, 'sprite:GFX_b#2'), null, 'an unconfigured table names its images under a base no host serves');
  assert.equal(iconImage(icons, 'sprite:GFX_c#3'), null);
  assert.equal(iconImage(icons, null), null);
});

test('a widget saved on a retired breakdown view is refused by name, never drawn as the full table under its old title', () => {
  registerWidgets([definition]);
  const saved = (show: string) => ({ ...createWidget('production-lines', 'track', 'lines'), options: { show } });
  assert.match(widgetUnavailable(saved('efficiency')) ?? '', /efficiency breakdown/);
  assert.match(widgetUnavailable(saved('output')) ?? '', /output breakdown/);
  assert.equal(widgetUnavailable(createWidget('production-lines', 'track', 'lines')), null);
});
