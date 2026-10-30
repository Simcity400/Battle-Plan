import assert from 'node:assert/strict';
import test from 'node:test';
import type { KeyedRun } from '../../dashboard/keyedHistory';
import { divisionEquipmentAt, historyAnswer, partAt, REPLY_BUDGET, splitPart, validateDivisionEquipmentHistory, WHOLE } from './divisionEquipmentHistory.ts';
import type { DivisionEquipmentHistory } from './divisionEquipmentHistory';
import { validateDivisionEquipment } from './divisionEquipmentData.ts';

const run = (first: number, last: number, value: string, starts = false): KeyedRun => [first, last, value, starts];
const series = (name: string, ...runs: KeyedRun[]) => ({ name, runs });
const literal = (text: string) => ({ text: [text], tables: [] });

/** Hours 10 to 30 known, 31 not yet prepared; GER exists from 10 to 25 with one type and one template. */
function history(): DivisionEquipmentHistory {
  return validateDivisionEquipmentHistory({
    branchId: '1', tag: 'GER', countFor: 'owner',
    window: { firstHour: 10, lastHour: 40, known: [{ firstHour: 10, lastHour: 30 }], reads: [], series: [
      series('*|pool', run(10, 30, '8', true)),
      series('*|stock', run(10, 11, '2', true), run(12, 30, '11')),
      series('state', run(10, 25, '1', true)),
      series('refused', run(20, 21, '1')),
      series('divisions', run(10, 25, '2', true)),
      series('short', run(10, 14, '1', true), run(15, 25, '0')),
      series('unknown', run(10, 25, '0', true)),
      series('type|1|held', run(10, 25, '200', true)),
      series('type|1|needed', run(10, 25, '300', true)),
      series('type|1|filled', run(10, 25, '200', true)),
      series('type|1|missing', run(10, 25, '100', true)),
      series('type|1|short', run(10, 25, '1', true)),
      series('type|1|unknown', run(10, 25, '0', true)),
      series('type|1|stock', run(12, 25, '900')),
      series('type|1|variant|70', run(10, 25, '150', true)),
      series('type|1|design|70', run(10, 25, '0', true)),
      series('type|1|variant|9', run(10, 25, '50', true)),
      series('template|5|divisions', run(10, 25, '2', true)),
      series('template|5|unknown', run(10, 25, '0', true)),
      series('template|5|typesShort', run(10, 25, '1', true)),
      series('template|5|manpower', run(10, 13, '1000', true), run(14, 25, '900')),
      series('template|5|hp', run(10, 25, '2000000', true)),
      series('template|5|naming', run(10, 25, '0', true)),
      series('template|5|artwork', run(10, 25, '0', true)),
    ] },
    kinds: [{ id: '1', archetype: 'widgets', name: 'Widgets', nameMissing: null }],
    namings: { 0: { name: 'Line division', nameMissing: null } },
    artworks: { 0: { icon: null, iconMissing: literal('Template artwork operands not recorded') } },
    designs: { 0: 'Synthetic model' },
    hours: [{ firstHour: 10, lastHour: 25, refusals: [{ reason: literal('Template artwork operands not recorded'), times: 1 }], fallback: 'Visual mods not shown: 1 file(s)' }],
    absent: ['Names unavailable: none'], served: ['Content read outside the recording'], warnings: ['a shader line no reached reply has'],
  })!;
}

test('an hour the history holds is the reached reply, measure by measure', () => {
  const reply = validateDivisionEquipment(divisionEquipmentAt(history(), 14));
  assert.deepEqual(reply, {
    branchId: '1', hour: 14, tag: 'GER', countFor: 'owner', absent: false, poolHour: 8, divisions: 2, divisionsShort: 1, unknownDivisions: 0,
    types: [{ archetype: 'widgets', name: 'Widgets', nameMissing: null, heldRaw: '200', neededRaw: '300', filledRaw: '200', missingRaw: '100',
      divisionsShort: 1, unknownDivisions: 0, stockpileRaw: '900', stockpileHour: 11,
      variants: [{ variant: '9', name: null, heldRaw: '50' }, { variant: '70', name: 'Synthetic model', heldRaw: '150' }] }],
    templates: [{ template: '5', name: 'Line division', nameMissing: null, icon: null, iconMissing: 'Template artwork operands not recorded',
      divisions: 2, manpower: '900', manpowerRequired: null, hpRaw: '2000000', hpMaxRaw: null, neededRaw: null, heldRaw: null,
      typesShort: 1, unknownDivisions: 0 }],
    warnings: ['Template artwork operands not recorded', 'Content read outside the recording', 'Visual mods not shown: 1 file(s)'],
  });
  assert.equal(divisionEquipmentAt(history(), 15)?.divisionsShort, 0);
  assert.equal(divisionEquipmentAt(history(), 11)?.types[0].stockpileRaw, null, 'stock not read then stays unknown, not zero');
  assert.equal(divisionEquipmentAt(history(), 11)?.types[0].stockpileHour, null, 'a read withholding the stock dates none');
});

test('a kept refusal names the hour each reply is for, counted and ordered by its words', () => {
  const base = history();
  const unavailable = { text: ['Template name unavailable: ', ' is unavailable: hour 3 Divisions \u0001'], tables: ['DivisionTemplateIcons'] };
  const kept = validateDivisionEquipmentHistory({
    ...base,
    namings: { 0: { name: null, nameMissing: unavailable } },
    hours: [{ firstHour: 10, lastHour: 25, refusals: [
      { reason: { text: ['', ' is unavailable: x'], tables: ['DivisionTemplateIcons'] }, times: 2 },
      { reason: literal('hour 11 DivisionTemplateIcons is unavailable: x'), times: 1 },
      { reason: literal('Zebra'), times: 1 },
      { reason: literal('\u{1F600} after'), times: 1 },
      { reason: literal('\uFFFD before'), times: 1 },
    ], fallback: null }],
  })!;
  for (const hour of [11, 24]) {
    const reply = divisionEquipmentAt(kept, hour)!;
    // The kept words stay as written; only the generated start names the hour.
    assert.equal(reply.templates[0].nameMissing, `Template name unavailable: hour ${hour} DivisionTemplateIcons is unavailable: hour 3 Divisions \u0001`);
    // At 11 the kept refusal words what a literal one does, and the two are counted together.
    const named = hour === 11 ? ['hour 11 DivisionTemplateIcons is unavailable: x (3)']
      : ['hour 11 DivisionTemplateIcons is unavailable: x', 'hour 24 DivisionTemplateIcons is unavailable: x (2)'];
    // By code point: U+FFFD before U+1F600, which UTF-16 order would swap.
    assert.deepEqual(reply.warnings, ['Zebra', ...named, '\uFFFD before', '\u{1F600} after', 'Content read outside the recording']);
  }
});

test('a country absent at a known hour is absent; unknown and refused hours ask the reached product', () => {
  const absent = divisionEquipmentAt(history(), 27);
  assert.equal(absent?.absent, true);
  assert.deepEqual(absent?.warnings, ['Names unavailable: none', 'Content read outside the recording'], 'no artwork drawn, no history-wide lines');
  assert.equal(absent?.poolHour, 8);
  assert.equal(divisionEquipmentAt(history(), 31), null, 'not prepared yet');
  assert.equal(divisionEquipmentAt(history(), 9), null);
  assert.equal(divisionEquipmentAt(history(), 20), null, 'the reached product answers with its own error');
});

test('a pending history is null and a malformed one is refused', () => {
  assert.equal(validateDivisionEquipmentHistory(null), null);
  assert.throws(() => validateDivisionEquipmentHistory({ ...history(), kinds: 'none' }));
});

test('the reached product is asked only for an hour the loaded history does not hold', () => {
  const loaded = { data: history() };
  assert.deepEqual(historyAnswer(true, loaded, 14).asked, false);
  assert.equal(historyAnswer(true, loaded, 14).held?.hour, 14);
  assert.equal(historyAnswer(true, loaded, 31).asked, true, 'not prepared yet');
  assert.equal(historyAnswer(true, loaded, 20).asked, true, 'refused: the reached product answers with its error');
  assert.equal(historyAnswer(true, {}, 14).asked, false, 'the history is still loading');
  assert.equal(historyAnswer(true, { data: null }, 14).asked, true, 'a pending history');
  assert.equal(historyAnswer(true, { error: 'too large' }, 14).asked, true);
  assert.deepEqual(historyAnswer(false, loaded, 14), { held: null, asked: true }, 'no prepared queries');
});

test('a count the history lacks, a malformed handle or an over-budget reply asks the reached product', () => {
  const without = (name: string) => { const held = history(); return { ...held, window: { ...held.window, series: held.window.series.filter(series => series.name !== name) } }; };
  assert.equal(divisionEquipmentAt(without('divisions'), 14), null, 'never zero');
  assert.equal(divisionEquipmentAt(without('type|1|short'), 14), null);
  assert.equal(divisionEquipmentAt(without('template|5|typesShort'), 14), null);
  const held = history();
  const odd = { ...held, window: { ...held.window, series: [...held.window.series, series('type|1|variant|x', run(10, 25, '1', true))] } };
  assert.equal(divisionEquipmentAt(odd, 14), null);
  const huge = { ...held, hours: [{ firstHour: 10, lastHour: 25, refusals: [{ reason: literal('x'.repeat(REPLY_BUDGET)), times: 1 }], fallback: null }] };
  assert.equal(divisionEquipmentAt(huge, 14), null);
});

test('a history answers only inside its range, and a range over budget is asked as halves', () => {
  const held = history();
  const windowed = { ...held, window: { ...held.window, firstHour: 12, lastHour: 20 } };
  assert.equal(divisionEquipmentAt(windowed, 11), null);
  assert.equal(divisionEquipmentAt(windowed, 21), null);
  assert.equal(divisionEquipmentAt(windowed, 14)?.hour, 14);
  const branch = { firstHour: 10, lastHour: 40 };
  assert.deepEqual(splitPart(WHOLE[0], 'This product reply exceeds the transport reply budget of 25165824 bytes', branch),
    [{ firstHour: null, lastHour: 25 }, { firstHour: 26, lastHour: null }]);
  assert.equal(splitPart(WHOLE[0], 'Recording changed', branch), null);
  assert.equal(splitPart({ firstHour: 12, lastHour: 12 }, 'exceeds the transport reply budget', branch), null);
  assert.equal(partAt([{ firstHour: null, lastHour: 25 }, { firstHour: 26, lastHour: null }], 30)?.firstHour, 26);
});
