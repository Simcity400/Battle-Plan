import assert from 'node:assert/strict';
import test from 'node:test';
import { liveKey, valuesOf } from '../../charts/live.ts';
import { efficiencyPercent, historyLanes, iconImage, lineKeyAt, lineValues, liveTotalText, productionLineKey, productionLineToken, totalText, validateAtReply, validateProductionReply, variantName, variantOf, weeklyRaw } from './productionData.ts';
import { campaignToken } from '../../dashboard/chosen.ts';
import { setToken } from '../../dashboard/options.ts';
import type { LineNow, ProductionLinesAtReply, ProductionLinesReply } from '../../generated/dashboard';

const day = (hour: number) => `h${hour}`;
const part = (units: string, exact = true) => ({ variant: '200', mio: '0', units, exact });

const reply: ProductionLinesReply = {
  branchId: '1', tag: 'GER', decimals: 5, detailHours: 720, warnings: [], icons: [{ key: 'sprite:a#1', image: '/api/artwork/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png' }], organisations: [],
  lineHours: [{ firstHour: 1000, lastHour: 1300 }],
  variants: [
    { id: '100', hour: 1300, known: true, token: 'infantry_equipment', name: 'Rifle', typeName: 'Infantry Equipment', nameUnknown: null, icon: 'sprite:a#1', iconUnknown: null },
    { id: '200', hour: 1199, known: false, token: '', name: null, typeName: null, nameUnknown: 'The variant has no definition at this hour.', icon: null, iconUnknown: 'x' },
  ],
  lines: [
    { key: '1#1', handle: '1', kind: 'military', beforeRecording: true, endedBy: null, finalOutput: null, steps: [
      { firstHour: 1000, lastHour: 1099, startsSegment: true, variant: '100', activeFactories: 2, requestedFactories: 3 },
      { firstHour: 1100, lastHour: 1239, startsSegment: false, variant: '100', activeFactories: 3, requestedFactories: 3 },
      { firstHour: 1251, lastHour: 1300, startsSegment: true, variant: '100', activeFactories: 3, requestedFactories: 3 },
    ] },
    { key: '2#1', handle: '2', kind: 'military', beforeRecording: false, endedBy: 1200, steps: [{ firstHour: 1050, lastHour: 1199, startsSegment: true, variant: '200', activeFactories: 1, requestedFactories: 1 }],
      finalOutput: { firstHour: 1200, lastHour: 1223, parts: [part('7')], sinceHour: 1000, wholeLife: true } },
  ],
};

test('each unbroken stretch of a line is a span whose band is its factories, and an ended line is grey with its final count', () => {
  const lanes = historyLanes(reply, 1300, true, day);
  assert.deepEqual(lanes.map(lane => lane.id), ['1#1', '2#1']);
  const first = lanes[0];
  assert.deepEqual(first.spans.map(span => [span.firstHour, span.lastHour]), [[1000, 1239], [1251, 1300]], 'a break in the reads splits the span');
  assert.deepEqual(first.spans[0].band, [{ firstHour: 1000, lastHour: 1099, value: 2 }, { firstHour: 1100, lastHour: 1239, value: 3 }]);
  assert.equal(first.label, '3 factories');
  assert.equal(first.identity?.icon, '/api/artwork/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png');
  const ended = lanes[1];
  assert.equal(ended.muted, true);
  assert.equal(ended.label, 'Ended by h1200 · 7 made');
  assert.equal(ended.identity?.label, 'Variant 200', 'a variant without a definition shows its id, never a blank');
  assert.deepEqual(historyLanes(reply, 1300, false, day).map(lane => lane.id), ['1#1']);
  assert.deepEqual(historyLanes(reply, 1040, true, day).map(lane => lane.id), ['1#1'], 'a line not yet seen has no lane');
});

test('a count with an uncounted tick is unknown, and no count is not recorded, never zero', () => {
  assert.equal(totalText({ parts: [part('3'), part('4')] }), '7');
  assert.equal(totalText({ parts: [part('3'), part('4', false)] }), 'Unknown');
  assert.equal(totalText(null), 'Not recorded');
});

const now = (handle: string, priority: number): LineNow => ({ handle, kind: 'military', variant: '100', mio: '0', maxFactories: 10, priority,
  output: { parts: [{ variant: '100', mio: '0', exact: true }], sinceHour: 1000, wholeLife: true } });

test('a running line paints its numbers from its cells, exact as the game computes them, and a cell without a value is missing, never zero', () => {
  const at: ProductionLinesAtReply = { branchId: '1', tag: 'GER', decimals: 5, missing: null, outputMissing: null, variants: [], organisations: [], icons: [], warnings: [],
    lines: [now('9', 0), now('11', 1)] };
  assert.deepEqual(validateAtReply(at), at);
  const values = valuesOf(new Map([[liveKey('9', 'active'), 4], [liveKey('9', 'efficiency'), 1_638_400], [liveKey('9', 'speed'), 200_000], [liveKey('9', 'cost'), 700_000],
    [liveKey('9', 'output'), 12]]), 'ProductionLines was not recorded at this hour');
  const read = lineValues(values, '9');
  assert.equal(read.active, 4);
  assert.equal(efficiencyPercent(read.efficiency!), 50n);
  assert.equal(weeklyRaw(read.speed, read.cost), 200_000n, 'speed × 700000 ÷ cost, truncated');
  assert.equal(weeklyRaw(1n, 3n), 233_333n);
  assert.equal(weeklyRaw(1n, 0n), null);
  assert.equal(liveTotalText(at.lines[0].output, read.output, null), '12');
  assert.equal(liveTotalText({ ...at.lines[0].output!, parts: [{ variant: '100', mio: '0', exact: false }] }, read.output, null), 'Unknown');
  assert.equal(liveTotalText(at.lines[0].output, read.output, 'Units made are not recorded at this hour'), 'Not recorded');
  const missing = lineValues(values, '11');
  assert.equal(missing.active, null);
  assert.equal(weeklyRaw(missing.speed, missing.cost), null);
  assert.equal(liveTotalText(at.lines[1].output, missing.output, null), '—');
  assert.equal(lineKeyAt(reply, '2', 1100), '2#1');
  assert.equal(lineKeyAt(reply, '2', 1250), null);
});

test('replies that break their shape are refused', () => {
  assert.deepEqual(validateProductionReply(reply), reply);
  assert.throws(() => validateProductionReply({ ...reply, lines: [reply.lines[0], reply.lines[0]] }), /Invalid production lines/);
  assert.throws(() => validateProductionReply({ ...reply, lines: [{ ...reply.lines[0], steps: [...reply.lines[0].steps].reverse() }] }), /Invalid production lines/);
  assert.equal(variantName(variantOf(reply.variants, '100'), '100'), 'Rifle');
});

test('a picture is drawn only from an artwork URL; an image no host serves is missing art, never a broken link', () => {
  const served = `http://artwork.localhost/${'0123456789abcdef'.repeat(2)}.png`;
  const icons = [{ key: 'sprite:GFX_a#1', image: served }, { key: 'sprite:GFX_b#2', image: `artwork:${'0123456789abcdef'.repeat(2)}.png` }];
  assert.equal(iconImage(icons, 'sprite:GFX_a#1'), served);
  assert.equal(iconImage(icons, 'sprite:GFX_b#2'), null, 'an unconfigured table names its images under a base no host serves');
  assert.equal(iconImage(icons, 'sprite:GFX_c#3'), null);
  assert.equal(iconImage(icons, null), null);
});

test('a chosen line reads back the key it was written with, whatever the country is named, and an older tag-based token still reads', () => {
  assert.equal(productionLineKey('c', '1', productionLineToken('c', '1', '42#1')), '42#1');
  assert.equal(productionLineKey('c', '1', setToken(campaignToken('c', JSON.stringify(['1', 'GER', '42#1'])))), '42#1', 'a token saved with the tag of its day');
  assert.equal(productionLineKey('c', '2', productionLineToken('c', '1', '42#1')), null, 'another branch');
  assert.equal(productionLineKey('other', '1', productionLineToken('c', '1', '42#1')), null, 'another campaign');
});

test('a line shows its recorded maximum factories, and output counted since a later start is the figure marked partial', async () => {
  const { productionLinesProjection } = await import('./productionLinesProjection.ts');
  const { createBoundedReadView } = await import('../../families/boundedView.ts');
  const at: ProductionLinesAtReply = { branchId: '1', tag: 'GER', decimals: 5, missing: null, outputMissing: null, variants: [], organisations: [], icons: [], warnings: [],
    lines: [{ ...now('9', 0), maxFactories: 15, output: { parts: [{ variant: '100', mio: '0', exact: true }], sinceHour: 1000, wholeLife: false } }] };
  const live = valuesOf(new Map([[liveKey('9', 'active'), 4], [liveKey('9', 'requested'), 6], [liveKey('9', 'output'), 12]]), 'not recorded');
  const model = productionLinesProjection.project(createBoundedReadView({ hour: 1200, panels: new Map([['main', at]]), live: new Map([['main', live]]) }), {});
  const cells = model.rows[0]!.cells;
  assert.deepEqual([cells.factories!.value, cells.factories!.secondaryValue], [{ kind: 'known', value: 4 }, { kind: 'known', value: 15 }], 'assigned / the recorded maximum, never the requested count');
  assert.equal(cells.total!.value.kind, 'partial');
  assert.equal(cells.total!.value.kind === 'partial' && cells.total!.value.value, 12);
});
