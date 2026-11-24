import assert from 'node:assert/strict';
import test from 'node:test';
import { firstHourOfMonth } from '../../dashboard/calendar.ts';
import { columnsExtent } from '../../charts/columns.ts';
import { plotFrame } from '../../charts/frame.ts';
import { valuesOf } from '../../charts/live.ts';
import type { ChartTokens } from '../../charts/tokens';
import type { ArmyLossesData } from './armyLossesData';
import type { LossArmy } from '../../generated/dashboard';
import { INDEPENDENT, LEDGERS, UNRESOLVED, armyChartPresentation, armyColumnsLayer, availabilityMessage, chartView, hatchKeyText, measureAvailabilityReason, monthArmies, monthBars, monthTooltip, monthView, resolveArmyLosses, validateArmyLosses } from './armyLossesData.ts';

const M = 1940 * 12;
const army = (reference: string, parent: string | null, parentMissing: string | null = null): LossArmy => ({ reference, ordinal: Number(reference),
  name: `Army ${reference}`, nameMissing: null, parent, parentName: parent ? `Group ${parent}` : null, parentMissing });
const reply = (): ArmyLossesData => ({ hour: firstHourOfMonth(M + 3), tag: 'GER', absent: false,
  armies: [army('1', '9'), army('2', null), army('3', null, 'not read')],
  manpower: [
    { key: '', army: '1', ordinal: 0, month: M, date: firstHourOfMonth(M), losses: [100, 20, 3] },
    { key: '', army: '2', ordinal: 0, month: M, date: firstHourOfMonth(M), losses: [-5, 0, 0] },
    { key: '', army: '3', ordinal: 0, month: M + 2, date: firstHourOfMonth(M + 2), losses: [7, 0, 0] },
    { key: '', army: 'gone', ordinal: 0, month: M, date: firstHourOfMonth(M), losses: [1000, 0, 0] },
  ],
  equipment: [
    { key: '', army: '1', ledger: 1, month: M, date: 0, variant: 'a', name: 'Rifle', archetype: 'rifle', archetypeName: 'Rifles', amountRaw: '200000' },
    { key: '', army: '1', ledger: 3, month: M, date: 0, variant: 'a', name: 'Rifle', archetype: 'rifle', archetypeName: 'Rifles', amountRaw: '100000' },
    { key: '', army: '1', ledger: 4, month: M, date: 0, variant: 'a', name: 'Rifle', archetype: 'rifle', archetypeName: 'Rifles', amountRaw: '900000' },
  ],
  emptyMonths: [], unknownMonths: [], availability: { armies: null, manpower: null, months: null, ownEquipment: null, enemyEquipment: null }, warnings: [] });
const span = [firstHourOfMonth(M), firstHourOfMonth(M + 3) + 5] as const;

test('channels stack Ground, Air and Navy signed, and months without rows are unknown without claiming expiry', () => {
  const data = reply();
  data.manpower.push({ key: '', army: '1', ordinal: 0, month: M + 3, date: firstHourOfMonth(M + 3), losses: [0, 0, 0] });
  const bars = monthBars(data, 'manpower', 'channel', 'combat', span[0], span[1], 10);
  assert.deepEqual(bars.parts.map(part => part.label), ['Ground', 'Air', 'Navy']);
  assert.equal(bars.periods.length, 4);
  assert.deepEqual(bars.periods[0].parts.map(part => part.value), [95, 20, 3], 'only counted armies; a negative correction is kept');
  assert.deepEqual(bars.periods[0].values, [95, 20, 3], 'the column bands are the parts, in order');
  assert.deepEqual(bars.periods[1].values, [], 'a month without rows has no bands');
  assert.equal(bars.periods[1].unknown, true, 'a month without rows uses the shared unknown column marker');
  assert.equal(bars.periods[1].noRows, true);
  assert.equal('unretained' in bars.periods[1], false);
  assert.equal(columnsExtent([bars.periods[1]], [bars.periods[1].firstHour, bars.periods[1].lastHour]), null,
    'an unknown month is excluded from numeric scaling');
  assert.deepEqual(monthTooltip(bars.periods[1], bars.parts, 'manpower', 'Manpower lost', null, () => '').rows,
    [{ label: 'No loss rows recorded for this month', tone: 'muted' }]);
  assert.equal(hatchKeyText(bars.periods), 'No loss rows recorded for this month');
  assert.deepEqual(bars.periods[2].parts.map(part => part.value), [7, 0, 0]);
  assert.equal(bars.periods[3].unknown, undefined, 'a recorded zero month stays numeric zero, never hatched');
  assert.equal(bars.periods[3].noRows, undefined);
  assert.deepEqual(bars.periods[3].values, [0, 0, 0]);
  assert.deepEqual(columnsExtent([bars.periods[3]], [bars.periods[3].firstHour, bars.periods[3].lastHour]), [0, 0]);

  const missingTable = { ...reply(), availability: { ...reply().availability, manpower: 'table not recorded' } };
  const unavailableBars = monthBars(missingTable, 'manpower', 'channel', 'combat', span[0], span[1], 10);
  assert.equal(unavailableBars.periods[1].unknown, true, 'when the table itself is unavailable, absent months stay unknown');
  assert.equal(unavailableBars.periods[1].noRows, undefined);
  assert.deepEqual(monthTooltip(unavailableBars.periods[1], unavailableBars.parts, 'manpower', 'Manpower lost', null, () => '').rows,
    [{ label: 'Not recorded', tone: 'muted' }]);
  assert.equal(hatchKeyText(unavailableBars.periods), 'Not recorded: these months’ totals are unknown');
});

test('unavailable equipment read hides chart only when no drawable months exist, otherwise keeps witnessed months with note', () => {
  const unavailableOnly = { ...reply(), equipment: [], availability: { ...reply().availability, ownEquipment: 'not recorded in a fast recording' } };
  const unavailableReason = measureAvailabilityReason(unavailableOnly, 'equipment', 'combat');
  assert.equal(unavailableReason, 'not recorded in a fast recording');
  assert.equal(measureAvailabilityReason(unavailableOnly, 'equipment', 'enemy'), null);
  assert.equal(availabilityMessage(unavailableReason!), 'The combat log wasn’t recorded at this time: not recorded in a fast recording.');
  const emptyBars = monthBars(unavailableOnly, 'equipment', 'type', 'combat', span[0], span[1], 10);
  assert.deepEqual(armyChartPresentation(emptyBars.periods, unavailableReason), {
    kind: 'unavailable',
    message: 'The combat log wasn’t recorded at this time: not recorded in a fast recording.',
  });

  const partial = { ...reply(), availability: { ...reply().availability, ownEquipment: 'SliceReads unavailable' } };
  const partialReason = measureAvailabilityReason(partial, 'equipment', 'attrition');
  const partialBars = monthBars(partial, 'equipment', 'type', 'attrition', span[0], span[1], 10);
  assert.equal(partialBars.periods[0].unknown, undefined, 'witnessed equipment month remains drawable');
  assert.equal(partialBars.periods[1].unknown, true, 'unwitnessed month remains unknown');
  assert.equal(partialBars.periods[1].noRows, undefined);
  assert.deepEqual(armyChartPresentation(partialBars.periods, partialReason), {
    kind: 'chart',
    note: 'The combat log wasn’t recorded at this time: SliceReads unavailable.',
  });
});

test('a recorded bar at the first hour of a month stays drawable when draw-in ends at the month boundary', () => {
  const bars = monthBars(reply(), 'manpower', 'channel', 'combat', span[0], firstHourOfMonth(M + 2), 10);
  const boundary = firstHourOfMonth(M + 2);
  const frame = plotFrame(600, 200, { left: 40, right: 20, top: 20, bottom: 20 }, [span[0], boundary], [0, 120], false);
  const rects: { x: number; y: number; width: number; height: number; fillStyle: string; globalAlpha: number }[] = [];
  const ctx = {
    fillStyle: '',
    strokeStyle: '',
    globalAlpha: 1,
    lineWidth: 1,
    save() {},
    restore() {},
    beginPath() {},
    rect() {},
    clip() {},
    moveTo() {},
    lineTo() {},
    stroke() {},
    fillRect(x: number, y: number, width: number, height: number) {
      rects.push({ x, y, width, height, fillStyle: this.fillStyle, globalAlpha: this.globalAlpha });
    },
  };
  const tokens = { muted: '#888', hatchSpacing: 4, lineWidth: { grid: 1, series: 2, emphasis: 3 } } as unknown as ChartTokens;
  armyColumnsLayer(bars.periods, ['#111', '#222', '#333']).draw(ctx as unknown as CanvasRenderingContext2D, frame, tokens, { playhead: boundary });
  const plotRight = frame.plotLeft + frame.plotWidth;
  const terminalBars = rects.filter(rect => rect.globalAlpha === 1 && rect.x + rect.width === plotRight);
  assert.equal(terminalBars.length, 1, 'terminal month at midnight draws its recorded bar inside the plot');
  assert.ok(terminalBars[0].width >= 1 && terminalBars[0].x >= frame.plotLeft);
  assert.equal(terminalBars[0].fillStyle, '#111');
  assert.ok(terminalBars[0].height > 0);
  const firstMonthBars = rects.filter(rect => rect.globalAlpha === 1 && rect.x < frame.plotLeft + 50);
  assert.equal(firstMonthBars.length, 3, 'interior months still draw all bands normally');

  const signedFrame = plotFrame(600, 200, { left: 40, right: 20, top: 20, bottom: 20 }, [span[0], span[1]], [-50, 120], false);
  const signedRects: { y: number; height: number; globalAlpha: number }[] = [];
  const signedCtx = {
    fillStyle: '', strokeStyle: '', globalAlpha: 1, lineWidth: 1,
    save() {}, restore() {}, beginPath() {}, rect() {}, clip() {}, moveTo() {}, lineTo() {}, stroke() {},
    fillRect(_x: number, y: number, _w: number, height: number) { signedRects.push({ y, height, globalAlpha: this.globalAlpha }); },
  };
  armyColumnsLayer(bars.periods, ['#111', '#222', '#333']).draw(signedCtx as unknown as CanvasRenderingContext2D, signedFrame, tokens, { playhead: null });
  const unknownHatches = signedRects.filter(rect => rect.globalAlpha === 0.25);
  assert.ok(unknownHatches.length >= 1, 'unknown months hatch');
  assert.equal(unknownHatches[0].y, signedFrame.plotTop);
  assert.equal(unknownHatches[0].height, signedFrame.plotHeight, 'army layer opts into fullHeightUnknown on signed axes');
});

test('army-group stacks use the current parent, an independent bucket and an explicit unresolved one', () => {
  const bars = monthBars(reply(), 'manpower', 'armyGroup', 'combat', span[0], span[1], 10);
  const parts = new Map(bars.parts.map(part => [part.key, part.label]));
  assert.equal(parts.get('9'), 'Group 9');
  assert.equal(parts.get(INDEPENDENT), 'Independent armies');
  assert.equal(parts.get(UNRESOLVED), 'Army group not recorded');
  const first = new Map(bars.periods[0].parts.map(part => [part.key, part.value]));
  assert.equal(first.get('9'), 123);
  assert.equal(first.get(INDEPENDENT), -5);
});

test('attrition adds its three modes, and recovery stays its own ledger', () => {
  const attrition = monthBars(reply(), 'equipment', 'type', 'attrition', span[0], span[1], 10);
  assert.deepEqual(attrition.periods[0].parts, [{ key: JSON.stringify(['archetype', 'rifle']), value: 3, raw: 300000n }]);
  const recovered = monthBars(reply(), 'equipment', 'type', 'recovered', span[0], span[1], 10);
  assert.deepEqual(recovered.periods[0].parts, [{ key: JSON.stringify(['archetype', 'rifle']), value: 9, raw: 900000n }]);
  assert.equal(monthBars(reply(), 'equipment', 'type', 'combat', span[0], span[1], 10).periods[0].unknown, true);
});

test('many parts fold the smallest into Other without losing their amount', () => {
  const data = reply();
  data.equipment = Array.from({ length: 30 }, (_, i) => ({ key: '', army: '1', ledger: 0, month: M, date: 0, variant: String(i), name: null,
    archetype: `type${i}`, archetypeName: null, amountRaw: String((i + 1) * 100000) }));
  const bars = monthBars(data, 'equipment', 'type', 'combat', span[0], span[1], 8);
  assert.equal(bars.parts.length, 8);
  assert.equal(bars.parts.at(-1)!.label, 'Other');
  assert.equal(bars.periods[0].parts.reduce((sum, part) => sum + part.value, 0), (30 * 31) / 2);
});

test('manpower totals wrap as the game’s signed 32-bit counters do', () => {
  const data = reply();
  data.armies = [army('1', '9'), army('2', '9')];
  data.manpower = [
    { key: '', army: '1', ordinal: 0, month: M, date: firstHourOfMonth(M), losses: [2147483647, 1, 0] },
    { key: '', army: '2', ordinal: 0, month: M, date: firstHourOfMonth(M), losses: [2147483647, 0, 0] },
  ];
  const grouped = monthBars(data, 'manpower', 'armyGroup', 'combat', span[0], span[1], 10);
  assert.deepEqual(grouped.periods[0].parts, [{ key: '9', value: -1, raw: -1n }], 'an army’s channels and the group’s armies both wrap');
  data.armies = [army('1', '9')];
  const channels = monthBars(data, 'manpower', 'channel', 'combat', span[0], span[1], 10);
  const total = monthTooltip(channels.periods[0], channels.parts, 'manpower', 'Manpower lost', null, () => '').rows.at(-1)!;
  assert.equal(total.value, '-2,147,483,648');
});

test('a month any counted army holds unwitnessed amounts for is unknown, not a partial total', () => {
  const data = reply();
  data.unknownMonths = [{ army: '2', ledger: 2, month: M, date: 0, reason: null }, { army: 'gone', ledger: 1, month: M + 2, date: 0, reason: null }];
  data.emptyMonths = [{ army: '1', ledger: 1, month: M + 2, date: 0, reason: null }];
  const attrition = monthBars(data, 'equipment', 'type', 'attrition', span[0], span[1], 10);
  assert.equal(attrition.periods[0].unknown, true, "army 2's attrition this month was not witnessed");
  assert.equal(attrition.periods[2].unknown, undefined, 'an army that is not counted changes nothing');
  assert.equal(monthBars(data, 'equipment', 'type', 'recovered', span[0], span[1], 10).periods[0].unknown, undefined,
    'another ledger stays known');
});

test('equipment subtotals stay exact through grouping, the Other fold and the tooltip total', () => {
  const data = reply();
  data.equipment = [60000, 30000, 10000].map((amount, i) => ({ key: '', army: '1', ledger: 0, month: M, date: 0, variant: String(i), name: null,
    archetype: `type${i}`, archetypeName: null, amountRaw: String(amount) }));
  const folded = monthBars(data, 'equipment', 'type', 'combat', span[0], span[1], 2);
  assert.deepEqual(folded.periods[0].parts.map(part => part.raw), [60000n, 40000n], 'Other adds 0.3 and 0.1 exactly');
  const tip = monthTooltip(folded.periods[0], folded.parts, 'equipment', 'Lost in combat', null, () => '');
  assert.equal(tip.rows.at(-1)!.value, '1', '0.6 + 0.3 + 0.1 is one item, never 0.9999999999999999');
  const grouped = monthBars(data, 'equipment', 'armyGroup', 'combat', span[0], span[1], 10);
  assert.deepEqual(grouped.periods[0].parts, [{ key: '9', value: 1, raw: 100000n }]);
});

test('the month drill keeps unknown ledgers unknown and armies that kept only an empty month', () => {
  const losses = { ...reply(), emptyMonths: [{ army: '2', ledger: 0, month: M, date: 0, reason: null }], unknownMonths: [{ army: '3', ledger: 0, month: M, date: 0, reason: null }] };
  const combat = monthArmies(losses, M, LEDGERS.combat);
  assert.deepEqual(combat.map(row => [row.army.reference, row.amount]), [['1', 0n], ['2', 0n], ['3', null]]);
  assert.deepEqual(monthArmies(losses, M, LEDGERS.attrition).map(row => row.amount), [300000n, 0n, 0n]);
  assert.deepEqual(monthArmies(losses, M, LEDGERS.enemy).map(row => row.amount), [null, null, null], 'no army kept the month in this ledger');
});

test('mod archetypes named like a bucket stay their own parts', () => {
  const data = reply();
  data.equipment = [['unclassified', 'Mod unclassified'], [null, null], ['other', 'Mod other'], ['a', 'A'], ['b', 'B']].map(([archetype, archetypeName], i) =>
    ({ key: '', army: '1', ledger: 0, month: M, date: 0, variant: String(i), name: null, archetype, archetypeName, amountRaw: String((10 - i) * 100000) }));
  const bars = monthBars(data, 'equipment', 'type', 'combat', span[0], span[1], 4);
  assert.deepEqual(bars.parts.map(part => part.label), ['Mod unclassified', 'Equipment type unavailable', 'Mod other', 'Other']);
  assert.equal(new Set(bars.parts.map(part => part.key)).size, 4, 'the real "other" archetype and the folded remainder are two parts');
});

test('amounts come from the reply\'s live cells, and a channel or amount without one leaves its month unknown', () => {
  const structure = validateArmyLosses({ tag: 'GER', absent: false, armies: [army('1', null)],
    manpower: [{ army: '1', ordinal: 0, month: M, date: firstHourOfMonth(M), key: 'm:1' }, { army: '1', ordinal: 1, month: M + 1, date: firstHourOfMonth(M + 1), key: 'm:2' }],
    equipment: [{ army: '1', ledger: 0, month: M, date: 0, design: 0, key: 'e:1' }],
    designs: [{ variant: 'a', name: 'Rifle I', archetype: 'rifle', archetypeName: 'Rifles' }],
    emptyMonths: [], unknownMonths: [], availability: { armies: null, manpower: null, months: null, ownEquipment: null, enemyEquipment: null }, warnings: [] });
  const cells = new Map([['m:1\u0000ground', 4], ['m:1\u0000air', 5], ['m:1\u0000navy', 6], ['m:2\u0000ground', 1], ['e:1\u0000amount', 250000]]);
  const data = resolveArmyLosses(structure, 7, valuesOf(cells, 'Not read yet'));
  assert.deepEqual(data.manpower.map(row => row.losses), [[4, 5, 6], null]);
  assert.equal(data.equipment[0].amountRaw, '250000');
  assert.deepEqual([data.equipment[0].name, data.equipment[0].archetype, data.equipment[0].archetypeName], ['Rifle I', 'rifle', 'Rifles'], 'a row names its design from the list');
  const bars = monthBars(data, 'manpower', 'channel', 'combat', firstHourOfMonth(M), firstHourOfMonth(M + 1) + 5, 10);
  assert.deepEqual(bars.periods.map(period => period.unknown ?? false), [false, true], 'a month with a channel not read is never a smaller total');
});

test('the chart asks only the measure it draws, and the drill one month of every part', () => {
  assert.deepEqual(chartView('manpower', 'attrition'), { kind: 'manpower' });
  assert.deepEqual(chartView('equipment', 'attrition'), { kind: 'equipment', ledgers: [1, 2, 3] });
  assert.deepEqual(chartView('equipment', 'enemy'), { kind: 'equipment', ledgers: [5] });
  assert.deepEqual(monthView(M), { kind: 'month', month: M });
});

test('a reply whose equipment names a design it does not list is refused', () => {
  const reply = { tag: 'GER', absent: false, armies: [], manpower: [], equipment: [{ army: '1', ledger: 0, month: M, date: 0, design: 1, key: 'e:1' }],
    designs: [{ variant: 'a', name: null, archetype: null, archetypeName: null }], emptyMonths: [], unknownMonths: [],
    availability: { armies: null, manpower: null, months: null, ownEquipment: null, enemyEquipment: null }, warnings: [] };
  assert.throws(() => validateArmyLosses(reply), /Invalid army losses reply/);
  assert.doesNotThrow(() => validateArmyLosses({ ...reply, equipment: [{ ...reply.equipment[0], design: 0 }] }));
});
