import assert from 'node:assert/strict';
import test from 'node:test';
import { liveKey, valuesOf } from '../../charts/live.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { breakdownAt, efficiencyPercent, iconImage, validateAtReply, validateBreakdownReply, variantName, variantOf, weeklyRaw } from './productionData.ts';
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
});

test('a row carries its line and country, so a freshly drawn table opens the breakdown at the clicked hour without any history', () => {
  const row = project(at([now('9', 0)]), new Map()).rows[0]!;
  assert.deepEqual(row.meta, { handle: '9', country: 'GER', name: 'Rifle' } satisfies ProductionLineMeta);
  const group = (id: string, totalRaw: string) => ({ id, label: id, totalRaw, decimals: 5, unit: '%', parts: [{ id: 'base', label: 'Base', raw: totalRaw, unknown: null }], note: '' });
  const breakdown: ProductionLineBreakdownReply = { branchId: '1', tag: 'GER', key: '9#1', firstHour: 720, lastHour: 1439, groups: [{ id: 'efficiency', label: 'Efficiency' }], warnings: [],
    runs: [{ firstHour: 720, lastHour: 999, groups: [group('efficiency', '1000')], warnings: [] }, { firstHour: 1000, lastHour: 1439, groups: [group('efficiency', '2000')], warnings: [] }] } as ProductionLineBreakdownReply;
  assert.deepEqual(validateBreakdownReply(breakdown), breakdown);
  assert.equal(breakdownAt(breakdown, 1200, 'efficiency')?.totalRaw, '2000', 'the run holding the clicked hour, not the window’s first');
  assert.equal(breakdownAt(breakdown, 1200, 'output'), null);
  assert.equal(breakdownAt(breakdown, 1500, 'efficiency'), null);
});

test('a picture is drawn only from an artwork URL; an image no host serves is missing art, never a broken link', () => {
  const served = `http://artwork.localhost/${'0123456789abcdef'.repeat(2)}.png`;
  const icons = [{ key: 'sprite:GFX_a#1', image: served }, { key: 'sprite:GFX_b#2', image: `artwork:${'0123456789abcdef'.repeat(2)}.png` }];
  assert.equal(iconImage(icons, 'sprite:GFX_a#1'), served);
  assert.equal(iconImage(icons, 'sprite:GFX_b#2'), null, 'an unconfigured table names its images under a base no host serves');
  assert.equal(iconImage(icons, 'sprite:GFX_c#3'), null);
  assert.equal(iconImage(icons, null), null);
});
