import assert from 'node:assert/strict';
import test from 'node:test';
import { laneStops } from '../../charts/timeline/layout.ts';
import { fitTooltip, tooltipText } from '../../charts/tooltip.ts';
import type { LineDetail, LineHistory, LineRun, LineStep, OutputRun, ProductionLineDetailReply, ProductionLinesReply, VariantName } from '../../generated/dashboard';
import { bandScale, detailOf, efficiencyFraction, efficiencyPercent, factoryScale, finalAt, finalTooltip, gapTooltip, historyLanes, iconImage, laneStatus, lineTooltip,
  linesAt, linesKnown, organisationAt, organisationText, outputAt, outputTotal, stepTooltip, totalNote, totalText, validateDetailReply,
  unreadNote, validateProductionReply, variantAt, variantName, variantParts, weeklyOutput, weeklyText, windowOf, withNote } from './productionData.ts';

const run = (firstHour: number, lastHour: number, values: Partial<LineRun> = {}): LineRun => ({
  firstHour, lastHour, startsSegment: false, variant: '70', mio: '79', activeFactories: 8, requestedFactories: 10, maxFactories: 150,
  costRaw: '50000', speedRaw: '310000', weeklyRaw: '4340000', efficiencyRaw: '1736704', priority: 0, amount: -1, ...values,
});
const output = (firstHour: number, lastHour: number, parts: [string, string, boolean][], values: Partial<OutputRun> = {}): OutputRun => ({
  firstHour, lastHour, parts: parts.map(([variant, units, exact]) => ({ variant, mio: '79', units, exact })), sinceHour: 100, wholeLife: false, ...values,
});

/** A line's steps as the product joins its runs: while the variant and factories
 * stay the same over consecutive hours of one stretch. */
function stepsOf(runs: readonly LineRun[]): LineStep[] {
  const steps: LineStep[] = [];
  runs.forEach((item, index) => {
    const previous = runs[index - 1];
    const last = steps.at(-1);
    if (last && previous && !item.startsSegment && previous.lastHour + 1 === item.firstHour && previous.variant === item.variant
      && previous.activeFactories === item.activeFactories && previous.requestedFactories === item.requestedFactories) last.lastHour = item.lastHour;
    else steps.push({ firstHour: item.firstHour, lastHour: item.lastHour, startsSegment: item.startsSegment, variant: item.variant,
      activeFactories: item.activeFactories, requestedFactories: item.requestedFactories });
  });
  return steps;
}

/** A line as both products send it: its history, and its reads (for the window). */
function line(key: string, runs: LineRun[], values: Partial<LineHistory> & { output?: OutputRun[] } = {}): { history: LineHistory; detail: LineDetail } {
  const { output: outputs = [], ...rest } = values;
  return {
    history: { key, handle: key.split('#')[0], kind: 'military', steps: stepsOf(runs), finalOutput: null, endedBy: null, beforeRecording: true,
      betweenReads: null, variants: [], ...rest },
    detail: { key, runs, output: outputs },
  };
}
const variant = (id: string, firstHour: number, name: string | null, values: Partial<VariantName> = {}): VariantName => ({
  id, firstHour, known: true, token: 'infantry_equipment_1', name, typeName: 'Infantry Equipment I', icon: null, iconUnknown: null, ...values,
});
const day = (hour: number) => `day ${hour}`;
const PNG = 'data:image/png;base64,AAAA';

const LINES = [
  line('5#1', [run(100, 123, { startsSegment: true }), run(124, 147), run(148, 200, { variant: '71', costRaw: '80000', weeklyRaw: '2712500', priority: 1, efficiencyRaw: '1769472' })], {
    output: [output(100, 123, [['70', '0', true]]), output(124, 147, [['70', '6', true]]), output(148, 200, [['70', '6', true], ['71', '4', true]])],
  }),
  line('6#1', [run(124, 147, { startsSegment: true, activeFactories: 2, requestedFactories: 3, speedRaw: '130000', priority: 1, mio: '0' }),
    run(148, 171, { activeFactories: 3, requestedFactories: 3, speedRaw: '130000', priority: 0, mio: '0' })], {
    endedBy: 172, beforeRecording: false,
    output: [output(124, 147, [['70', '2', true]], { wholeLife: true }), output(148, 171, [['70', '5', false]], { wholeLife: true })],
    finalOutput: output(172, 172, [['70', '7', true]], { wholeLife: true }),
  }),
];

const REPLY: ProductionLinesReply = {
  branchId: '1', tag: 'GER', decimals: 5, reads: [100, 124, 148], outputHours: [{ firstHour: 100, lastHour: 200 }], detailHours: 720,
  lineHours: [{ firstHour: 100, lastHour: 150 }, { firstHour: 160, lastHour: 200 }], organisationHours: [{ firstHour: 100, lastHour: 190 }],
  unreadOutput: [], warnings: [],
  lines: LINES.map(item => item.history),
  variants: [
    variant('70', 100, 'Infantry Equipment I', { icon: 'sprite:GFX_infantry_weapons1_medium' }),
    variant('71', 100, 'Gewehr 98', { iconUnknown: 'the creator’s ruling party is not recorded at this hour' }),
    variant('71', 160, 'Gewehr 98k', { icon: 'sprite:GFX_GER_rifle_medium' }),
  ],
  organisations: [{ id: '79', firstHour: 100, token: 'GER_mauser_organization', name: 'Mauser', icon: 'sprite:GFX_idea_mauser', nameUnknown: null, iconUnknown: null }],
  icons: [{ key: 'sprite:GFX_infantry_weapons1_medium', image: PNG }, { key: 'sprite:GFX_GER_rifle_medium', image: null }, { key: 'sprite:GFX_idea_mauser', image: PNG }],
};

const DETAIL: ProductionLineDetailReply = { branchId: '1', tag: 'GER', decimals: 5, window: 0, firstHour: 0, lastHour: 719, lines: LINES.map(item => item.detail) };

test('well-formed replies pass and malformed ones are refused', () => {
  assert.equal(validateProductionReply(REPLY), REPLY);
  assert.equal(validateDetailReply(DETAIL), DETAIL);
  const steps = (key: string, list: LineStep[]) => ({ ...LINES[0].history, key, steps: list });
  assert.throws(() => validateProductionReply({ ...REPLY, lines: [steps('5#1', [...stepsOf([run(100, 130)]), ...stepsOf([run(120, 140)])])] }));
  assert.throws(() => validateProductionReply({ ...REPLY, lines: [REPLY.lines[0], REPLY.lines[0]] }), 'keys are unique');
  assert.throws(() => validateProductionReply({ ...REPLY, icons: [{ key: 'a', image: 'http://x' }] }));
  assert.throws(() => validateProductionReply({ ...REPLY, reads: [124, 100] }));
  assert.throws(() => validateProductionReply({ ...REPLY, detailHours: 0 }));
  assert.throws(() => validateDetailReply({ ...DETAIL, lines: [{ key: '5#1', runs: [run(100, 130), run(120, 140)], output: [] }] }));
  assert.throws(() => validateDetailReply({ ...DETAIL, lines: [{ key: '5#1', runs: [run(100, 130, { efficiencyRaw: '1.5' })], output: [] }] }));
  assert.throws(() => validateDetailReply({ ...DETAIL, lines: [{ key: '5#1', runs: [], output: [output(100, 110, [['70', 'x', true]])] }] }));
});

test('the history holds each line’s factory steps; a window holds its reads', () => {
  // 5#1's reads at 100 and 124 differ only in values the band does not draw: one step.
  assert.deepEqual(REPLY.lines[0].steps.map(step => [step.firstHour, step.lastHour, step.variant]), [[100, 147, '70'], [148, 200, '71']]);
  assert.equal(windowOf(REPLY, 719), 0);
  assert.equal(windowOf(REPLY, 720), 1);
  assert.equal(detailOf(DETAIL, '6#1')?.runs.length, 2);
  assert.equal(detailOf(DETAIL, '9#1'), null);
  assert.equal(detailOf(undefined, '5#1'), null);
});

test('output per week is the product’s figure by the game’s weekly formula', () => {
  // 3.1 IC a day at 0.5 a unit: 43.4 a week.
  assert.equal(weeklyOutput(run(0, 1)), 4_340_000n);
  assert.equal(weeklyOutput(run(0, 1, { weeklyRaw: null })), null);
  assert.equal(weeklyText(4_340_000n, 5), '43.4');
  // A ship at 4.125 IC a day costing 11,456 makes 0.00252 a week: never shown as 0.
  assert.equal(weeklyText(252n, 5), '0.00252');
  assert.equal(weeklyText(0n, 5), '0');
});

test('efficiency is the recorded Q15 value rounded as the production row shows it', () => {
  // 53% exactly, and 0x4000 (half a percent) rounds up.
  assert.equal(efficiencyPercent(run(0, 1, { efficiencyRaw: String(53 * 32768) })), 53n);
  assert.equal(efficiencyPercent(run(0, 1, { efficiencyRaw: String(53 * 32768 + 0x3fff) })), 53n);
  assert.equal(efficiencyPercent(run(0, 1, { efficiencyRaw: String(53 * 32768 + 0x4000) })), 54n);
  assert.equal(efficiencyFraction(run(0, 1, { efficiencyRaw: '1638400' })), 0.5);
  assert.equal(efficiencyFraction(run(0, 1, { efficiencyRaw: '9999999' })), 1);
});

test('lines at an hour come in the game’s order, by factories or by IC a day', () => {
  assert.deepEqual(linesAt(REPLY, DETAIL, 130, 'game').map(at => at.line.key), ['5#1', '6#1']);
  assert.deepEqual(linesAt(REPLY, DETAIL, 150, 'game').map(at => at.line.key), ['6#1', '5#1']);
  assert.deepEqual(linesAt(REPLY, DETAIL, 150, 'factories').map(at => at.line.key), ['5#1', '6#1']);
  assert.deepEqual(linesAt(REPLY, DETAIL, 150, 'output').map(at => at.line.key), ['5#1', '6#1']);
  assert.deepEqual(linesAt(REPLY, DETAIL, 180, 'game').map(at => at.line.key), ['5#1'], 'an ended line is not running');
  assert.equal(factoryScale(REPLY), 10);
  assert.equal(bandScale(REPLY), 8);
});

test('variants, organisations and icons are as recorded at the hour, never from a later read', () => {
  assert.equal(variantName(variantAt(REPLY, '71', 150), '71'), 'Gewehr 98');
  assert.equal(variantName(variantAt(REPLY, '71', 170), '71'), 'Gewehr 98k');
  // A variant not recorded shows its id, never a blank.
  assert.equal(variantName(variantAt(REPLY, '99', 150), '99'), 'Variant 99');
  // Before its first definition read a variant is unknown, not its later name.
  const renamed = { ...REPLY, variants: [variant('72', 124, 'Gewehr 98'), variant('72', 148, 'Gewehr 98k')] };
  assert.equal(variantAt(renamed, '72', 100), null);
  assert.equal(variantName(variantAt(renamed, '72', 130), '72'), 'Gewehr 98');
  assert.equal(iconImage(REPLY, variantAt(REPLY, '70', 150)!.icon), PNG);
  // A picture that could not be read, and an unknown icon, show none.
  assert.equal(iconImage(REPLY, variantAt(REPLY, '71', 170)!.icon), null);
  assert.equal(iconImage(REPLY, variantAt(REPLY, '71', 150)!.icon), null);
  assert.equal(organisationText(organisationAt(REPLY, '79', 150), run(0, 1)), 'Mauser');
  assert.equal(organisationText(organisationAt(REPLY, '0', 150), run(0, 1, { mio: '0' })), '—');
  assert.equal(organisationText(organisationAt(REPLY, '80', 150), run(0, 1, { mio: '80' })), 'Not recorded');
  assert.equal(organisationText({ id: '79', firstHour: 0, token: 'GER_mauser_organization', name: null, icon: null, nameUnknown: null, iconUnknown: null }, run(0, 1)), 'GER_mauser_organization');
});

test('an unresolved organisation owner is said, instead of a generic name', () => {
  const unknown = { ...REPLY.organisations[0], token: 'acme', name: null, icon: null,
    nameUnknown: 'The owner is unresolved; its country-specific name is unknown.', iconUnknown: 'The owner is unresolved; its country-specific icon is unknown.' };
  const reply = { ...REPLY, organisations: [unknown] };
  assert.equal(validateProductionReply(reply), reply);
  assert.equal(organisationText(organisationAt(reply, '79', 100), run(100, 124)), 'Unknown owner');
  const at = linesAt(reply, DETAIL, 130, 'game')[0];
  for (const tip of lineTooltip(reply, at.line, at.detail, at.run, 130, {}, day)) {
    assert.equal(tip.rows.find(row => row.label === 'Military-industrial organisation')?.value, 'Unknown owner');
    assert.ok(tip.note?.includes(unknown.nameUnknown), 'the History tooltip explains the unknown owner');
    const spoken = tooltipText(tip);
    assert.match(spoken, /Military-industrial organisation Unknown owner/);
    assert.ok(spoken.includes(unknown.nameUnknown), 'the keyboard read-out keeps the reason');
    assert.ok(!spoken.includes('acme'), 'the script key is not presented as an organisation name');
  }
  assert.throws(() => validateProductionReply({ ...reply, organisations: [{ ...unknown, nameUnknown: 0 }] }));
});

test('a variant is unknown where the variant definitions were not recorded', () => {
  // Read at 100 as Gewehr 98; at 124 a census is read while the definitions are not,
  // so an unobserved rename or icon change could lie there: unknown, not the old name.
  const unknown = (id: string, firstHour: number) => variant(id, firstHour, null, { known: false, typeName: null, iconUnknown: 'The variant is not recorded at this time.' });
  const gap = { ...REPLY, variants: [variant('70', 100, 'Infantry Equipment I'), unknown('70', 124), variant('70', 148, 'Infantry Equipment I'),
    variant('71', 100, 'Gewehr 98'), unknown('71', 124), variant('71', 148, 'Gewehr 98')] };
  assert.equal(variantAt(gap, '71', 110)?.name, 'Gewehr 98');
  assert.equal(variantAt(gap, '71', 124), null);
  assert.equal(variantName(variantAt(gap, '71', 124), '71'), 'Variant 71');
  const at = linesAt(gap, DETAIL, 130, 'game')[0];
  const [tip] = lineTooltip(gap, at.line, at.detail, at.run, 130, {}, day);
  assert.equal(tip.title, 'Variant 70');
  assert.equal(tip.icon, undefined);
  assert.match(tip.note ?? '', /name is not recorded at this time/);
});

test('a line’s total is its recorded output: exact, not exact, or not recorded, never guessed', () => {
  const [five, six] = REPLY.lines;
  const at = (history: LineHistory, hour: number) => outputAt(history, detailOf(DETAIL, history.key), hour);
  assert.equal(totalText(at(five, 150)), '10');
  assert.equal(totalText(at(five, 90)), 'Not recorded');
  // A part not counted exactly makes the total no number at all.
  assert.equal(outputTotal(at(six, 150)!), null);
  assert.equal(totalText(at(six, 150)), 'Not exact');
  // After it ended, the final count published then, with or without the window.
  assert.equal(totalText(at(six, 180)), '7');
  assert.equal(totalText(outputAt(six, null, 180)), '7');
  assert.match(totalNote(at(five, 150), day)!, /since day 100, when recording began/);
  assert.match(totalNote(at(six, 130), day)!, /since the line was created/);
  assert.match(totalNote(at(six, 150), day)!, /not exact/);
});

test('the line tooltip shows what the game shows, with each variant’s part of a switched line', () => {
  const [first] = linesAt(REPLY, DETAIL, 150, 'factories');
  const [tip] = lineTooltip(REPLY, first.line, first.detail, first.run, 150, { subtitle: 'day 150' }, day);
  assert.equal(tip.title, 'Gewehr 98');
  const rows = Object.fromEntries(tip.rows.filter(row => row.value).map(row => [row.label, row.value]));
  assert.deepEqual(rows, { Factories: '8 of 10 requested', Efficiency: '54%', Output: '27.1 per week', Cost: '0.8 IC per unit', Priority: '2',
    Ordered: 'No limit', 'Military-industrial organisation': 'Mauser', 'Produced so far': '10', 'Infantry Equipment I': '6', 'Gewehr 98': '4' });
  assert.equal(tip.rows.find(row => row.label === 'Military-industrial organisation')?.icon, PNG);
  assert.match(tip.note ?? '', /since day 100/);
});

test('a tooltip with many parts fits the window, and the full breakdown is still read out', () => {
  // A line whose output names 50 variants: every part row would not fit a short window.
  const parts: [string, string, boolean][] = Array.from({ length: 50 }, (_, index) => [String(100 + index), String(index + 1), true]);
  const many = line('7#1', [run(100, 200)], { output: [output(100, 200, parts)] });
  const forms = lineTooltip(REPLY, many.history, many.detail, run(100, 200), 150, {}, day);
  assert.equal(forms[0].rows.filter(row => row.indent).length, 50);
  const shown = fitTooltip(forms, 600, { full: forms[0] });
  assert.ok(shown.rows.length < forms[0].rows.length, 'the eye gets a form that fits');
  assert.ok(shown.rows.some(row => row.label === 'Of 50 variants'), 'with the parts counted, not silently cut');
  // Assistive technology reads every part.
  assert.match(tooltipText(shown), /Variant 149 50/);
  // A tall window shows every part.
  assert.equal(fitTooltip(forms, 4000, { full: forms[0] }).rows.length, forms[0].rows.length);
});

test('history lanes: one per line, a span per variant stretch with its factories as a band and its art, ended lines grey', () => {
  const lanes = historyLanes(REPLY, 200, true, day);
  assert.deepEqual(lanes.map(lane => [lane.id, lane.identity?.label, lane.label, lane.muted]), [
    ['5#1', 'Gewehr 98k', '8 factories', false],
    ['6#1', 'Infantry Equipment I', 'Ended by day 172', true],
  ]);
  assert.deepEqual(lanes[0].spans.map(span => [span.label, span.firstHour, span.lastHour, span.emphasis, span.band?.length, span.icon]),
    [['Infantry Equipment I', 100, 147, true, 1, PNG], ['Gewehr 98', 148, 200, true, 1, undefined]]);
  assert.equal(lanes[1].identity?.icon, PNG);
  assert.deepEqual(lanes[1].spans[0].band, [{ firstHour: 124, lastHour: 147, value: 2 }, { firstHour: 148, lastHour: 171, value: 3 }]);
  assert.deepEqual(historyLanes(REPLY, 200, false, day).map(lane => lane.id), ['5#1']);
  assert.deepEqual(historyLanes(REPLY, 110, true, day).map(lane => lane.id), ['5#1']);
});

test('the keyboard reaches every read of a line whose settings did not change', () => {
  // Settings unchanged from 100 to 171 (one step); its count read at 100, 124 and 148 is 0, 10 and 20.
  const steady = line('5#1', [run(100, 171, { startsSegment: true })], {
    output: [output(100, 123, [['70', '0', true]]), output(124, 147, [['70', '10', true]]), output(148, 171, [['70', '20', true]])],
  });
  const reply = { ...REPLY, lines: [steady.history] };
  const [lane] = historyLanes(reply, 171, true, day);
  const stops = laneStops(lane.spans, 0, 171, null);
  assert.deepEqual(stops.map(stop => stop.hour), [100, 124, 148]);
  // Each stop's tooltip gives that read's count.
  const detail = { ...DETAIL, lines: [steady.detail] };
  const counts = stops.map(stop => {
    const at = linesAt(reply, detail, stop.hour, 'game')[0];
    return lineTooltip(reply, at.line, at.detail, at.run, stop.hour, {}, day)[0].rows.find(row => row.label === 'Produced so far')?.value;
  });
  assert.deepEqual(counts, ['0', '10', '20']);
});

test('a line’s status beside its lane follows the hour shown', () => {
  const [five, six] = REPLY.lines;
  assert.deepEqual(laneStatus(six, 130, day), { text: '2 factories', ended: false });
  assert.deepEqual(laneStatus(six, 180, day), { text: 'Ended by day 172', ended: true });
  assert.deepEqual(laneStatus(five, 90, day), { text: 'Starts day 100', ended: false });
  const stopped = line('7#1', [run(100, 120)]).history;
  assert.deepEqual(laneStatus(stopped, 130, day), { text: 'Last recorded day 120', ended: true });
});

test('lines are unknown outside a recorded census, and none only where a census recorded none', () => {
  assert.equal(linesKnown(REPLY, 120), true);
  assert.equal(linesKnown(REPLY, 155), false, 'a gap in the recording');
  assert.equal(linesKnown(REPLY, 90), false, 'before the first census');
});

test('an ended lane keeps its final count and breakdown from its end onwards', () => {
  const six = REPLY.lines[1];
  // After it ended, hovering the lane gives the final count.
  const [after] = finalTooltip(REPLY, six, 180, { subtitle: 'day 180' }, day);
  assert.equal(after.rows[0].label, 'Ended by day 172');
  assert.deepEqual(after.rows.slice(1).map(row => [row.label, row.value]), [['Made in all', '7']]);
  // Its bars, which the keyboard reaches, carry it once the view is past its end, and not before.
  const bar = linesAt(REPLY, DETAIL, 130, 'game').find(at => at.line.key === '6#1')!;
  const [late] = lineTooltip(REPLY, bar.line, bar.detail, bar.run, 130, {}, day, 200);
  assert.ok(late.rows.some(row => row.label === 'Made in all' && row.value === '7'));
  const [early] = lineTooltip(REPLY, bar.line, bar.detail, bar.run, 130, {}, day, 150);
  assert.ok(!early.rows.some(row => row.label === 'Made in all'));
  assert.equal(finalAt(six, 171), null);
  assert.equal(finalAt(six, 172)?.parts[0].units, '7');
});

test('a line made and deleted between two reads gets a lane, its total, and a keyboard stop where it was published', () => {
  const between = line('9#1', [], {
    kind: null, endedBy: 124, beforeRecording: false, betweenReads: { firstHour: 101, lastHour: 124 }, variants: ['71', '70'],
    finalOutput: output(124, 124, [['71', '4', true], ['70', '1', true]], { wholeLife: true }),
  }).history;
  const reply = { ...REPLY, lines: [...REPLY.lines, between] };
  assert.equal(validateProductionReply(reply), reply);
  assert.throws(() => validateProductionReply({ ...REPLY, lines: [line('9#1', []).history] }), 'a line without reads needs its final count');
  const lanes = historyLanes(reply, 171, true, day);
  const lane = lanes.find(item => item.id === '9#1')!;
  assert.deepEqual([lane.label, lane.muted, lane.spans.map(span => [span.firstHour, span.lastHour, span.band])],
    ['Made and deleted by day 124', true, [[101, 124, undefined]]]);
  assert.deepEqual(historyLanes(reply, 200, false, day).map(item => item.id), ['5#1']);
  // In a history ending at 171 the keyboard reaches 124, where its count is published.
  const stops = laneStops(lane.spans, 0, 171, null).map(stop => stop.hour);
  assert.deepEqual(stops, [101, 124]);
  assert.equal(finalTooltip(reply, between, 101, {}, day)[0].rows.find(row => row.label === 'Made in all')?.value, 'Not recorded');
  const [tip] = finalTooltip(reply, between, stops[1], {}, day);
  assert.deepEqual(tip.rows.map(row => [row.label, row.value]), [
    ['Made and deleted between day 101 and day 124', undefined], ['Made in all', '5'], ['Gewehr 98', '4'], ['Infantry Equipment I', '1']]);
  assert.match(tip.note ?? '', /since the line was created/);
});

test('a final count shows only from the census that published it', () => {
  // A census at 124 first omits the line, output is unavailable, and its final count is published at 148.
  const late = line('8#1', [run(100, 123)], { endedBy: 124, finalOutput: output(148, 148, [['70', '9', true]], { wholeLife: true }) }).history;
  assert.equal(finalAt(late, 130), null, 'a range ending at 130 does not show a count published at 148');
  assert.equal(finalAt(late, 148)?.parts[0].units, '9');
  const [early] = finalTooltip(REPLY, late, 130, {}, day);
  assert.ok(!early.rows.some(row => row.value === '9'));
  assert.equal(early.rows.find(row => row.label === 'Made in all')?.value, 'Not recorded');
  assert.match(early.note ?? '', /recorded only from day 148/);
  assert.equal(finalTooltip(REPLY, late, 150, {}, day)[0].rows.find(row => row.label === 'Made in all')?.value, '9');
});

test('an organisation is unknown where the organisations were not recorded', () => {
  assert.equal(organisationAt(REPLY, '79', 150)?.name, 'Mauser');
  // Past the recorded hours (an unread rename could lie there): unknown, not the old name.
  assert.equal(organisationAt(REPLY, '79', 195), null);
  assert.equal(organisationText(organisationAt(REPLY, '79', 195), run(0, 1)), 'Not recorded');
});

test('a line ended before the first complete census is not said to be made between reads', () => {
  const before = line('5#1', [], {
    kind: null, endedBy: 108, beforeRecording: true, betweenReads: { firstHour: 100, lastHour: 108 }, variants: ['70'],
    finalOutput: output(108, 108, [['70', '3', true]]),
  }).history;
  assert.deepEqual(laneStatus(before, 120, day), { text: 'Ended by day 108', ended: true });
  const [tip] = finalTooltip(REPLY, before, 120, {}, day);
  assert.equal(tip.rows[0].label, 'Ran before recording began; ended by day 108');
  assert.match(tip.note ?? '', /since day 100, when recording began/);
});

test('while a window of reads loads, a history tooltip shows the line’s variant and factories and says so', () => {
  const [tip] = stepTooltip(REPLY, REPLY.lines[1].steps[0], 130, { subtitle: 'day 130' }, 'Loading its other values…');
  assert.equal(tip.title, 'Infantry Equipment I');
  assert.deepEqual(tip.rows.filter(row => row.value).map(row => [row.label, row.value]), [['Factories', '2 of 3 requested']]);
  assert.match(tip.note ?? '', /Loading/);
});

test('a gap in a lane says its lines are not recorded then, never the factories at the end shown', () => {
  // 5 factories 100–123, nothing recorded 124–147, 20 factories 148–200; the history ends at 200.
  const gapped = line('5#1', [run(100, 123, { startsSegment: true, activeFactories: 5 }), run(148, 200, { startsSegment: true, activeFactories: 20 })]).history;
  const reply = { ...REPLY, lines: [gapped] };
  const [lane] = historyLanes(reply, 200, true, day);
  assert.equal(lane.label, '20 factories', 'the lane is labelled at the end shown');
  const tip = gapTooltip(reply, gapped, 130, 'day 130', day);
  assert.deepEqual(tip.rows.map(row => row.label), ['Not recorded then']);
  assert.ok(!tipText(tip).includes('20'), 'nothing from another hour');
  assert.equal(tip.title, 'Infantry Equipment I');
  // Recorded hours show their own factories (a step's tooltip), before it started its status.
  assert.equal(stepTooltip(reply, gapped.steps[0], 110, {}, '')[0].rows.find(row => row.label === 'Factories')?.value, '5 of 10 requested');
  assert.equal(stepTooltip(reply, gapped.steps[1], 160, {}, '')[0].rows.find(row => row.label === 'Factories')?.value, '20 of 10 requested');
  assert.deepEqual(gapTooltip(reply, gapped, 90, 'day 90', day).rows.map(row => row.label), ['Starts day 100']);
});

const tipText = (tip: { title: string; rows: readonly { label: string; value?: string }[] }) => [tip.title, ...tip.rows.map(row => `${row.label} ${row.value ?? ''}`)].join(' ');

test('output parts of one variant under two organisations are one variant', () => {
  // 10 of variant 70 under organisation 79, then 20 of it under 80: one variant, 30 units.
  const twice: OutputRun = { ...output(100, 200, [['70', '10', true]]), parts: [
    { variant: '70', mio: '79', units: '10', exact: true }, { variant: '70', mio: '80', units: '20', exact: true }] };
  assert.deepEqual(variantParts(twice), [{ variant: '70', units: 30n }]);
  const one = line('5#1', [run(100, 200)], { output: [twice] });
  const [full, compact] = lineTooltip(REPLY, one.history, one.detail, run(100, 200), 150, {}, day);
  const made = full.rows.find(row => row.label === 'Produced so far');
  assert.equal(made?.value, '30');
  assert.ok(!full.rows.some(row => row.indent), 'no per-variant rows for one variant');
  assert.ok(!compact.rows.some(row => /Of \d+ variants/.test(row.label)));
  // A variant is exact only when every part of it is; others keep their own sums.
  const mixed: OutputRun = { ...twice, parts: [...twice.parts.slice(0, 1), { variant: '70', mio: '80', units: '20', exact: false },
    { variant: '71', mio: '79', units: '4', exact: true }] };
  assert.deepEqual(variantParts(mixed), [{ variant: '70', units: null }, { variant: '71', units: 4n }]);
  const split = line('6#1', [run(100, 200)], { output: [mixed] });
  const [rows, counted] = lineTooltip(REPLY, split.history, split.detail, run(100, 200), 150, {}, day);
  assert.deepEqual(rows.rows.filter(row => row.indent).map(row => [row.label, row.value]), [['Infantry Equipment I', 'Not exact'], ['Gewehr 98', '4']]);
  assert.ok(counted.rows.some(row => row.label === 'Of 2 variants'));
});

test('hours whose output read was lost say lines made and ended there may be missing', () => {
  // The read at 124 could not be read: 101–124 is marked.
  const reply = { ...REPLY, unreadOutput: [{ firstHour: 101, lastHour: 124 }] };
  assert.equal(validateProductionReply(reply), reply);
  assert.equal(unreadNote(reply, 100), undefined);
  assert.match(unreadNote(reply, 110) ?? '', /may be missing/);
  assert.equal(unreadNote(reply, 125), undefined);
  // It reaches every tooltip there, beside the read's own note.
  assert.equal(withNote('Read day 100.', unreadNote(reply, 110)), `Read day 100. ${unreadNote(reply, 110)}`);
  assert.equal(withNote(undefined, undefined), undefined);
  const at = linesAt(reply, DETAIL, 110, 'game')[0];
  const [tip] = lineTooltip(reply, at.line, at.detail, at.run, 110, { note: withNote(undefined, unreadNote(reply, 110)) }, day);
  assert.match(tip.note ?? '', /Lines made and ended here may be missing/);
  assert.match(gapTooltip(reply, reply.lines[0], 90, 'day 90', day, unreadNote(reply, 110)).note ?? '', /may be missing/);
  assert.throws(() => validateProductionReply({ ...REPLY, unreadOutput: [{ firstHour: 130, lastHour: 120 }] }));
});
