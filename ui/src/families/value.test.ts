import assert from 'node:assert/strict';
import test from 'node:test';
import { CELL } from '../dashboard/follow/message.ts';
import { parseFollowed } from '../test-support/followMessage.ts';
import { message, type Spec } from '../dashboard/follow/messages.fixture.ts';
import {
  InvalidFamilyValueError,
  assertValue,
  compareValueRows,
  compareValues,
  knownValue,
  textValue,
  notApplicable,
  notRecorded,
  partialValue,
  valueFromLive,
  answerRowIndex,
  valueFromNamedAnswer,
  valueFromNullable,
  valueFromRawBigInt,
  valueFromWireCell,
  wireFloat,
  wireWhole,
  zeroValue,
} from './value.ts';
import { canonicalAuditData } from './audit.ts';
import {
  GAME_WINDOW_MISSING_LABEL,
  auditValue,
  formatBoundDate,
  formatCompactRational,
  formatCompactScalar,
  formatDateHover,
  formatDateOnly,
  formatDateParts,
  formatDateValue,
  formatOutOf,
  formatProgress,
  formatRational,
  formatRecordedPercent,
  formatScalar,
  formatShare,
  formatShareRatio,
  formatValueState,
  notRecordedLabel,
  spokenValue,
  valueStateProps,
  type ValueFormat,
} from './valueFormat.ts';
import * as valueFormat from './valueFormat.ts';
import { engineHour } from '../dashboard/calendar.ts';

const answer = (spec: Spec) => parseFollowed(message(100, [{ id: 1, answer: spec }])).answers[0]!.answer;

test('value constructors enforce the five states and reject bare null, undefined, NaN and empty reasons', () => {
  assert.deepEqual(knownValue(42), { kind: 'known', value: 42 });
  assert.deepEqual(knownValue(0), { kind: 'zero', value: 0 });
  assert.deepEqual(knownValue(0n), { kind: 'zero', value: 0n });
  assert.deepEqual(zeroValue(0), { kind: 'zero', value: 0 });
  assert.deepEqual(zeroValue(0n), { kind: 'zero', value: 0n });
  assert.deepEqual(notRecorded('Unrecorded'), { kind: 'notRecorded', reason: 'Unrecorded' });
  assert.deepEqual(notApplicable('Not at war'), { kind: 'notApplicable', reason: 'Not at war' });
  assert.deepEqual(partialValue(12, 'Gaps excluded'), { kind: 'partial', value: 12, reason: 'Gaps excluded' });

  assert.throws(() => knownValue(null as unknown as number), InvalidFamilyValueError);
  assert.throws(() => knownValue(undefined as unknown as number), InvalidFamilyValueError);
  assert.throws(() => knownValue(Number.NaN), InvalidFamilyValueError);
  assert.throws(() => knownValue(Number.POSITIVE_INFINITY), InvalidFamilyValueError);
  assert.throws(() => knownValue(''), InvalidFamilyValueError);
  assert.throws(() => knownValue('   \t\n '), InvalidFamilyValueError);
  assert.throws(() => partialValue('', 'reason'), InvalidFamilyValueError);
  assert.throws(() => partialValue('  ', 'reason'), InvalidFamilyValueError);
  assert.throws(() => assertValue({ kind: 'known', value: '' }), InvalidFamilyValueError);
  assert.throws(() => assertValue({ kind: 'partial', value: '   ', reason: 'r' }), InvalidFamilyValueError);
  assert.throws(() => notRecorded(''), InvalidFamilyValueError);
  assert.throws(() => notApplicable('   '), InvalidFamilyValueError);
  assert.throws(() => partialValue(Number.NaN, 'reason'), InvalidFamilyValueError);
  assert.throws(() => partialValue(5, ''), InvalidFamilyValueError);
  assert.throws(() => assertValue(null, 'test cell'), /InvalidFamilyValueError|expected a 5-state family Value/);
  assert.throws(() => assertValue(undefined, 'test cell'), InvalidFamilyValueError);
  assert.throws(() => assertValue({ kind: 'known', value: Number.NaN }), InvalidFamilyValueError);
  assert.throws(() => assertValue({ kind: 'bogus' }), InvalidFamilyValueError);
});

test('wire and exact adapters map every wire state and nullable meaning into the five states', () => {
  const big = (1n << 60n) + 7n;
  const rows = answer({
    keys: [1n, 2n, 3n, 4n, 5n, 6n],
    columns: 1,
    names: ['GER', 'ENG', 'FRA', 'SOV', 'ITA', 'JAP'],
    values: [42n, 0n, 0n, 0n, 0n, big],
    states: [CELL.value, CELL.value, CELL.missing, CELL.unrecorded, CELL.noRow, CELL.value],
    reasons: [null, null, 'Missing in source', null, 'Not held', null],
  });
  assert.equal(rows.kind, 'rows');
  assert.deepEqual(valueFromWireCell(rows, 0, 0), { kind: 'known', value: 42 });
  assert.deepEqual(valueFromWireCell(rows, 1, 0), { kind: 'zero', value: 0 });
  assert.deepEqual(valueFromWireCell(rows, 2, 0), { kind: 'notRecorded', reason: 'Missing in source' });
  assert.deepEqual(valueFromWireCell(rows, 3, 0), { kind: 'notRecorded', reason: 'Not recorded at this hour' });
  assert.deepEqual(valueFromWireCell(rows, 4, 0), { kind: 'notRecorded', reason: 'Not held' });
  assert.deepEqual(valueFromWireCell(rows, 5, 0), { kind: 'known', value: big });

  assert.deepEqual(valueFromNamedAnswer(rows, 'GER'), { kind: 'known', value: 42 });
  assert.deepEqual(valueFromNamedAnswer(rows, 'ENG'), { kind: 'zero', value: 0 });
  assert.deepEqual(valueFromNamedAnswer(rows, 'FRA'), { kind: 'notRecorded', reason: 'Missing in source' });
  assert.deepEqual(valueFromNamedAnswer(rows, 'JAP'), { kind: 'known', value: big });
  assert.deepEqual(valueFromNamedAnswer(rows, 'USA'), { kind: 'notRecorded', reason: 'Not in the answer' });

  const nextHour = answer({
    keys: [1n, 2n, 3n, 4n, 5n, 6n],
    columns: 1,
    names: ['GER', 'ENG', 'FRA', 'SOV', 'ITA', 'JAP'],
    values: [43n, 1n, 0n, 0n, 0n, big],
    states: [CELL.value, CELL.value, CELL.value, CELL.unrecorded, CELL.noRow, CELL.value],
    reasons: [null, 'A reason that moves every text offset', null, null, 'Not held', null],
  });
  assert.equal(nextHour.kind === 'rows' && rows.kind === 'rows' && answerRowIndex(nextHour) === answerRowIndex(rows), true,
    'the next hour\'s answer with the same names reuses the row index: none is rebuilt');
  assert.deepEqual(valueFromNamedAnswer(nextHour, 'GER'), { kind: 'known', value: 43 });
  const renamed = answer({ keys: [1n, 2n], columns: 1, names: ['GER', 'ITA'], values: [1n, 2n], states: [CELL.value, CELL.value], reasons: [null, null] });
  const sameSize = answer({ keys: [1n, 2n], columns: 1, names: ['GER', 'ENG'], values: [1n, 2n], states: [CELL.value, CELL.value], reasons: [null, null] });
  assert.equal(renamed.kind === 'rows' && sameSize.kind === 'rows' && answerRowIndex(renamed) !== answerRowIndex(sameSize), true, 'different names build their own index');
  assert.deepEqual(valueFromNamedAnswer(sameSize, 'ENG'), { kind: 'known', value: 2 });
  assert.deepEqual(valueFromNamedAnswer(renamed, 'ENG'), { kind: 'notRecorded', reason: 'Not in the answer' });

  const refused = answer({ notRecorded: 'Table not recorded' });
  assert.deepEqual(valueFromNamedAnswer(refused, 'GER'), { kind: 'notRecorded', reason: 'Table not recorded' });
  assert.deepEqual(valueFromNamedAnswer(undefined, 'GER'), { kind: 'notRecorded', reason: 'Not read yet' });

  const floatBits = new DataView(new ArrayBuffer(8));
  floatBits.setFloat64(0, 12.5, true);
  const floatRows = answer({ keys: [1n], columns: 2, values: [floatBits.getBigInt64(0, true), 7n], states: [CELL.value, CELL.value] });
  assert.equal(floatRows.kind, 'rows');
  assert.deepEqual(wireFloat(floatRows, 0), { kind: 'known', value: 12.5 });
  assert.deepEqual(wireWhole(floatRows, 1), { kind: 'known', value: 7n });

  assert.deepEqual(valueFromNullable(null, { kind: 'notApplicable', reason: 'No garrison' }), { kind: 'notApplicable', reason: 'No garrison' });
  assert.deepEqual(valueFromNullable(null, { kind: 'notRecorded', reason: 'Omitted' }), { kind: 'notRecorded', reason: 'Omitted' });
  assert.deepEqual(valueFromNullable(0, { kind: 'notRecorded', reason: 'Omitted' }), { kind: 'zero', value: 0 });
  assert.deepEqual(valueFromNullable(5, { kind: 'notRecorded', reason: 'Omitted' }, { partial: 'Gaps' }), { kind: 'partial', value: 5, reason: 'Gaps' });
  assert.deepEqual(valueFromRawBigInt('0', { kind: 'notRecorded', reason: 'No value' }), { kind: 'zero', value: 0n });
  assert.deepEqual(valueFromRawBigInt('9007199254740993123', { kind: 'notRecorded', reason: 'No value' }), { kind: 'known', value: 9007199254740993123n });
  assert.throws(() => valueFromRawBigInt('not-a-bigint', { kind: 'notRecorded', reason: 'No value' }), InvalidFamilyValueError);

  const live = { get: (k: string) => (k === 'a' ? 0 : k === 'b' ? 9 : undefined), missing: () => 'Not live' };
  assert.deepEqual(valueFromLive(live, 'a'), { kind: 'zero', value: 0 });
  assert.deepEqual(valueFromLive(live, 'b', { partialReason: 'Subset' }), { kind: 'partial', value: 9, reason: 'Subset' });
  assert.deepEqual(valueFromLive(live, 'c'), { kind: 'notRecorded', reason: 'Not live' });
});

test('sorting places notRecorded and notApplicable last in both directions and breaks ties by name then key', () => {
  const rows = [
    { key: 'z', name: 'Zulu', value: notRecorded('Missing') },
    { key: 'b', name: 'Bravo', value: knownValue(10) },
    { key: 'a', name: 'Alpha', value: zeroValue(0) },
    { key: 'c2', name: 'Bravo', value: knownValue(10) },
    { key: 'c1', name: 'Bravo', value: knownValue(10) },
    { key: 'na', name: 'Aardvark', value: notApplicable('N/A') },
    { key: 'p', name: 'Partial', value: partialValue(25, 'Gaps') },
  ];

  const desc = [...rows].sort((x, y) => compareValueRows(x, y, 'descending')).map(r => r.key);
  assert.deepEqual(desc, ['p', 'b', 'c1', 'c2', 'a', 'na', 'z']);

  const asc = [...rows].sort((x, y) => compareValueRows(x, y, 'ascending')).map(r => r.key);
  assert.deepEqual(asc, ['a', 'b', 'c1', 'c2', 'p', 'na', 'z']);

  assert.equal(compareValues(knownValue(1n << 80n), knownValue((1n << 80n) + 1n), 'descending') > 0, true);
});

test('formatting and spoken forms render zero as 0, never None or dash, and carry reasons for absent and partial values', () => {
  const z1 = formatValueState(zeroValue(0), 'integer');
  assert.equal(z1.kind, 'zero');
  assert.equal(z1.text, '0');
  const z2 = formatValueState(zeroValue(0n), { decimals: 0 });
  assert.equal(z2.kind, 'zero');
  assert.equal(z2.text, '0');
  const nr = formatValueState(notRecorded('Unrecorded'), 'integer');
  assert.equal(nr.kind, 'notRecorded');
  assert.equal(nr.label, 'Not recorded');
  const na = formatValueState(notApplicable('Not at war'), 'integer');
  assert.equal(na.kind, 'notApplicable');
  assert.equal(na.label, '\u2014');

  const part = formatValueState(partialValue(120, 'Gaps excluded'), 'integer');
  assert.equal(part.kind, 'partial');
  assert.equal(part.text, '120');
  assert.equal(part.reason, 'Gaps excluded');

  assert.equal(spokenValue(zeroValue(0), 'integer'), '0');
  assert.equal(spokenValue(notRecorded('No sensor'), 'integer'), 'Not recorded: No sensor');
  assert.equal(spokenValue(notApplicable('Peacetime'), 'integer'), 'Not applicable: Peacetime');
  assert.equal(spokenValue(partialValue(15, 'Gaps'), 'integer'), '15, partial: Gaps');
  const partialAtLeast = formatValueState(partialValue(15, 'Gaps', { atLeast: true }), 'integer');
  assert.equal(partialAtLeast.kind, 'partial');
  assert.equal(partialAtLeast.spoken, '15, partial: Gaps');

  assert.equal(auditValue(zeroValue(0), 'integer'), JSON.stringify(['zero', ['int', '0']]));
  assert.equal(auditValue(notRecorded('No sensor'), 'integer'), JSON.stringify(['notRecorded', 'No sensor']));
  assert.equal(auditValue(notApplicable('Peacetime'), 'integer'), JSON.stringify(['notApplicable', 'Peacetime']));
  assert.equal(auditValue(partialValue(15, 'Gaps'), 'integer'), JSON.stringify(['partial', ['int', '15'], 'Gaps', false]));
  assert.notEqual(auditValue(zeroValue(0), 'decimal1'), auditValue(knownValue(0.04), 'decimal1'));
  assert.notEqual(auditValue(knownValue('1000')), auditValue(knownValue(1000)));
  assert.equal(auditValue(knownValue(1e21)), auditValue(knownValue(10n ** 21n)));
});

test('bigint formatting preserves exact precision beyond 2^53 in clockSeconds, decimal1, percent2, and maxDecimals', () => {
  const big = (1n << 60n) + 7n;
  assert.equal(spokenValue(knownValue(big), 'decimal1'), '1,152,921,504,606,846,983');
  assert.equal(spokenValue(knownValue(big), 'percent2'), '1,152,921,504,606,846,983%');
  assert.equal(spokenValue(knownValue(big), { decimals: 0, maxDecimals: 2 }), '1,152,921,504,606,846,983');
  const bigSeconds = 3600n * 100000000000000000n + 61n;
  assert.equal(spokenValue(knownValue(bigSeconds), 'clockSeconds'), '100000000000000000:01:01');

  assert.equal(spokenValue(knownValue(1e21), 'integer'), spokenValue(knownValue(10n ** 21n), 'integer'));
  assert.equal(spokenValue(knownValue(1e21), 'integer'), '1,000,000,000,000,000,000,000');
  assert.equal(spokenValue(knownValue(1e21), 'decimal1'), spokenValue(knownValue(10n ** 21n), 'decimal1'));
  assert.equal(spokenValue(knownValue(1e21), 'percent2'), spokenValue(knownValue(10n ** 21n), 'percent2'));
});

test('canonicalAuditData encodes Map and Set deterministically with tags and throws on unsupported container types', () => {
  const m1 = new Map<string, number>([['b', 2], ['a', 1]]);
  const m2 = new Map<string, number>([['a', 1], ['b', 2]]);
  const m3 = new Map<string, number>([['a', 1], ['b', 3]]);
  assert.equal(canonicalAuditData(m1), canonicalAuditData(m2));
  assert.notEqual(canonicalAuditData(m1), canonicalAuditData(m3));
  assert.notEqual(canonicalAuditData(m1), canonicalAuditData({}));

  const s1 = new Set(['z', 'a']);
  const s2 = new Set(['a', 'z']);
  const s3 = new Set(['a', 'b']);
  assert.equal(canonicalAuditData(s1), canonicalAuditData(s2));
  assert.notEqual(canonicalAuditData(s1), canonicalAuditData(s3));
  assert.notEqual(canonicalAuditData(s1), canonicalAuditData({}));

  class CustomBox { x: number; constructor(x: number) { this.x = x; } }
  assert.throws(() => canonicalAuditData(new Date(0)), /Unsupported audit value type: Date/);
  assert.throws(() => canonicalAuditData(/abc/), /Unsupported audit value type: RegExp/);
  assert.throws(() => canonicalAuditData(new CustomBox(1)), /Unsupported audit value type: CustomBox/);
  assert.throws(() => canonicalAuditData(Symbol('s')), /Unsupported audit value type: symbol/);
  assert.throws(() => canonicalAuditData(() => 1), /Unsupported audit value type: function/);
});

test('F4: canonicalAuditData recognises family values only by exact plain-object shape and never drops extra fields or skips prototype checks', () => {
  const recA = { kind: 'partial', value: 10, reason: 'gap', hour: 100 };
  const recB = { kind: 'partial', value: 10, reason: 'gap', hour: 200 };
  assert.notEqual(canonicalAuditData(recA), canonicalAuditData(recB), 'extra field hour is preserved and differentiates records');

  const knownExtraA = { kind: 'known', value: 5, extra: 'a' };
  const knownExtraB = { kind: 'known', value: 5, extra: 'b' };
  assert.notEqual(canonicalAuditData(knownExtraA), canonicalAuditData(knownExtraB), 'extra fields on known-like plain objects are not dropped');

  class FakePartial {
    kind = 'partial' as const;
    value = 1;
    reason = 'gap';
  }
  assert.throws(() => canonicalAuditData(new FakePartial()), /Unsupported audit value type: FakePartial/);
  assert.equal(
    canonicalAuditData(partialValue(10, 'gap', { atLeast: true })),
    JSON.stringify(['partial', ['int', '10'], 'gap', true]),
  );
});

test('M1: every value format avoids exponent notation for 1e21, 1e-7, and MAX_SAFE_INTEGER + 2', () => {
  const formats: ValueFormat[] = [
    'integer',
    'fixed5Percent',
    'fixed5Amount',
    'fixed5Width',
    'clockSeconds',
    'decimal1',
    'percent2',
    { decimals: 2 },
    { decimals: 2, percent: true },
    { decimals: 0, maxDecimals: 2 },
    { decimals: 0, maxDecimals: 2, signed: true },
  ];
  const samples = [1e21, 1e-7, Number.MAX_SAFE_INTEGER + 2];
  for (const format of formats) {
    for (const sample of samples) {
      const text = formatScalar(sample, format);
      assert.equal(/[eE][+-]?\d/.test(text), false, `format ${JSON.stringify(format)} on ${sample} produced exponent string: ${text}`);
    }
  }
  assert.equal(formatScalar(1e21, { decimals: 2 }), '10,000,000,000,000,000,000');
  assert.equal(formatScalar(1e21, { decimals: 2, percent: true }), '1,000,000,000,000,000,000,000%');
  assert.equal(formatScalar(1e21, { decimals: 0, maxDecimals: 2 }), '1,000,000,000,000,000,000,000');
  assert.equal(formatScalar(1e-7, { decimals: 2 }), '0');
  assert.equal(formatScalar(1e-7, 'decimal1'), '<0.1');
  assert.deepEqual([47484000n, 40950000n, 140000n, -149999n].map(raw => formatScalar(raw, { decimals: 5, maxDecimals: 0 })), ['474', '409', '1', '\u22121'], 'a scaled amount shown to fewer places truncates toward zero');
  assert.deepEqual([420000000n, 248975000n, 400000n, 170000n].map(raw => formatScalar(raw, 'fixed5Amount')), ['4,200', '2,489.75', '4', '1.7'], 'fixed5 amounts group thousands and show decimals only where they exist');
  assert.equal(formatScalar(1e-7, 'percent2'), '<0.01%');
  assert.equal(formatScalar(1e-7, { decimals: 0, maxDecimals: 2 }), '<0.01');
  assert.equal(formatScalar(Number.MAX_SAFE_INTEGER + 2, 'integer'), '9,007,199,254,740,992');
  assert.equal(formatScalar(Number.MAX_SAFE_INTEGER + 2, { decimals: 2 }), '90,071,992,547,409.92');
  assert.equal(formatScalar(1e21, 'clockSeconds'), '277777777777777777:46:40');
});

test('dates omit hours on screen, carry exact hours on hover, and format bounded and missing dates', () => {
  const hour = engineHour({ year: 1941, month: 9, day: 14, hour: 8 })!;
  assert.equal(formatDateOnly(hour), '14 Sep 1941');
  assert.equal(formatDateOnly(hour).includes(':'), false);
  assert.equal(formatDateHover(hour), '14 Sep 1941 · 08:00');
  assert.deepEqual(formatDateParts(hour), {
    dayMonth: '14 Sep',
    year: '1941',
    full: '14 Sep 1941',
    hover: '14 Sep 1941 · 08:00',
  });
  const bound = formatBoundDate(hour, 'by', 'First observed after recording started.');
  assert.equal(bound.display, '14 Sep 1941');
  assert.equal(bound.hover, 'By 14 Sep 1941 · 08:00');
  assert.equal(bound.reason, 'First observed after recording started.');

  const missingDate = formatDateValue(notRecorded('Date was absent from the log.'));
  assert.equal(missingDate.kind, 'notRecorded');
  assert.equal(missingDate.label, 'Date not recorded');

  const knownDate = formatDateValue(knownValue(hour), { bound: { kind: 'since', reason: 'Observed since gap end.' } });
  assert.equal(knownDate.kind, 'known');
  assert.equal(knownDate.text, '14 Sep 1941');
  assert.equal(knownDate.hover, 'Since 14 Sep 1941 · 08:00');
});

test('missing-value vocabulary, compact K/M formatting, and N / max preserve exact figures and missing caps', () => {
  assert.equal(notRecordedLabel(), 'Not recorded');
  assert.equal(notRecordedLabel('Manpower'), 'Manpower not recorded');
  assert.equal(GAME_WINDOW_MISSING_LABEL, 'n/a');

  const namedMissing = formatValueState(notRecorded('Sensor offline'), undefined, { figure: 'Fuel' });
  assert.equal(namedMissing.kind, 'notRecorded');
  assert.equal(namedMissing.label, 'Fuel not recorded');

  const windowMissing = formatValueState(notRecorded('Not read in this build'), undefined, { gameWindow: true });
  assert.equal(windowMissing.kind, 'notRecorded');
  assert.equal(windowMissing.label, 'n/a');
  assert.equal(windowMissing.gameWindow, true);

  const compact = formatCompactScalar(1_250_000, 'integer');
  assert.deepEqual(compact, { text: '1.25M', exact: '1,250,000', compacted: true });
  const small = formatCompactScalar(850, 'integer');
  assert.deepEqual(small, { text: '850', exact: '850', compacted: false });

  const compactState = formatValueState(knownValue(45_600), 'integer', { compact: true });
  assert.equal(compactState.kind, 'known');
  assert.equal(compactState.text, '45.6K');
  assert.equal(compactState.exact, '45,600');
  assert.equal(compactState.spoken, '45,600');

  const ratio = formatOutOf(knownValue(12), knownValue(15), 'integer');
  assert.equal(ratio.kind, 'known');
  assert.equal(ratio.text, '12 / 15');

  const missingCap = formatOutOf(knownValue(12), notRecorded('Cap not recorded'), 'integer');
  assert.equal(missingCap.kind, 'partial');
  assert.equal(missingCap.text, '12');
  assert.match(missingCap.partialReason!, /Cap not recorded/);

  const missingVal = formatOutOf(notRecorded('Count missing'), knownValue(15), 'integer');
  assert.equal(missingVal.kind, 'notRecorded');
  assert.equal(missingVal.missingReason, 'Count missing');
});


test('text from a product that is blank or absent is not recorded, with the reason', () => {
  assert.deepEqual(textValue('  ', 'Name not localised'), notRecorded('Name not localised'));
  assert.deepEqual(textValue(null, 'Name not localised'), notRecorded('Name not localised'));
  assert.deepEqual(textValue('Brest', 'Name not localised'), knownValue('Brest'));
});

test('fixedDecimals, formatShare, formatProgress, formatRecordedPercent, and signed truncation follow the display precision rules', () => {
  assert.equal(formatScalar(0, { decimals: 0, maxDecimals: 1, fixedDecimals: true, unit: '°C' }), '0.0 °C');
  assert.equal(formatScalar(-5.2, { decimals: 0, maxDecimals: 1, fixedDecimals: true, unit: '°C' }), '−5.2 °C');
  assert.equal(formatScalar(12, { decimals: 0, maxDecimals: 1, fixedDecimals: true, unit: '°C' }), '12.0 °C');
  assert.equal(formatScalar(15.5, { decimals: 0, maxDecimals: 1, percent: true, signed: true }), '+15.5%');
  assert.equal(formatScalar(-10.2, { decimals: 0, maxDecimals: 1, percent: true, signed: true }), '−10.2%');
  assert.equal(formatScalar(0, { decimals: 0, maxDecimals: 1, percent: true, signed: true }), '0%');
  assert.equal(formatScalar(0.4, { decimals: 0, signed: true }), '+<1');
  assert.equal(formatScalar(-0.4, { decimals: 0, signed: true }), '−<1');
  assert.equal(formatScalar(120.7, { decimals: 0, signed: true }), '+120');
  assert.equal(formatScalar(-120.7, { decimals: 0, signed: true }), '−120');
  assert.equal(formatScalar(-5, 'percent2'), '−5.00%');

  assert.equal(formatShare(0), '0%');
  assert.equal(formatShare(0.0001), '<0.1%');
  assert.equal(formatShare(0.0009), '<0.1%');
  assert.equal(formatShare(0.001), '0.1%');
  assert.equal(formatShare(0.03), '3.0%');
  assert.equal(formatShare(0.04), '4.0%');
  assert.equal(formatShare(0.042), '4.2%');
  assert.equal(formatShare(0.07), '7.0%');
  assert.equal(formatShare(0.073), '7.3%');
  assert.equal(formatShare(0.0999), '9.9%');
  assert.equal(formatShare(0.15), '15%');
  assert.equal(formatShare(0.334), '33%');
  assert.equal(formatShare(0.6667), '67%');
  assert.equal(formatShare(0.996), '99%');
  assert.equal(formatShare(0.9999), '99%');
  assert.equal(formatShare(1), '100%');
  assert.throws(() => formatShare(null as unknown as number), InvalidFamilyValueError);
  assert.throws(() => formatShare(Number.NaN), InvalidFamilyValueError);
  assert.throws(() => formatShare(-0.1), InvalidFamilyValueError);
  assert.throws(() => formatShare(1.25), InvalidFamilyValueError);
  assert.throws(() => formatProgress(null as unknown as number), InvalidFamilyValueError);
  assert.throws(() => formatProgress(Number.NaN), InvalidFamilyValueError);
  assert.throws(() => formatRecordedPercent(Number.NaN), InvalidFamilyValueError);
  assert.throws(() => formatScalar(Number.NaN, 'integer'), InvalidFamilyValueError);
  assert.throws(() => formatScalar(null as unknown as number, 'integer'), InvalidFamilyValueError);
  assert.throws(() => formatCompactScalar(Number.NaN, 'integer'), InvalidFamilyValueError);
  assert.equal(formatValueState(notRecorded('Missing share'), 'share').kind, 'notRecorded');
  assert.equal(formatValueState(knownValue(0.0004), 'share').kind, 'known');
  const tinyShare = formatValueState(knownValue(0.0004), 'share');
  assert.equal(tinyShare.kind === 'known' ? tinyShare.text : '', '<0.1%');

  assert.equal(formatScalar(255_000n, { decimals: 5, maxDecimals: 1, fixedDecimals: true, unit: '°C' }), '2.5 °C');
  assert.equal(formatScalar(260_000n, { decimals: 5, maxDecimals: 1, fixedDecimals: true, unit: '°C' }), '2.6 °C');
  assert.equal(formatScalar(101_999n, { decimals: 5, maxDecimals: 2 }), '1.01');
  assert.equal(formatScalar(4_000n, { decimals: 5, maxDecimals: 1 }), '<0.1');
  assert.equal(formatScalar(-4_000n, { decimals: 5, maxDecimals: 1, fixedDecimals: true, unit: '°C' }), '−<0.1 °C');
  assert.equal(formatScalar(4_000n, { decimals: 5, maxDecimals: 1, signed: true }), '+<0.1');
  assert.equal(formatScalar(0.04, 'decimal1'), '<0.1');
  assert.equal(formatScalar(-0.04, { decimals: 0, maxDecimals: 1, unit: '°C' }), '−<0.1 °C');
  assert.equal(formatScalar(-1.5, { decimals: 0, signed: true }), '−1');
  assert.equal(formatScalar(1000, { decimals: 0, unit: 'days' }), '1,000 days');
  assert.equal(formatScalar(1000, 'percent2'), '1,000.00%');
  assert.deepEqual(formatCompactScalar(7_139, 'integer'), { text: '7.13K', exact: '7,139', compacted: true });
  assert.deepEqual(formatCompactScalar(999_999, 'integer'), { text: '999K', exact: '999,999', compacted: true });
  assert.deepEqual(formatCompactScalar(1_439, 'integer'), { text: '1.43K', exact: '1,439', compacted: true });

  assert.equal(formatProgress(0), '0%');
  assert.equal(formatProgress(0.009), '0%');
  assert.equal(formatProgress(0.29), '29%');
  assert.equal(formatProgress(0.57), '57%');
  assert.equal(formatProgress(0.58), '58%');
  assert.equal(formatProgress(0.625), '62%');
  assert.equal(formatProgress(0.996), '99%');
  assert.equal(formatProgress(0.999999999), '99%');
  assert.equal(formatProgress(1), '100%');
  assert.equal(formatScalar(99.6, 'progress'), '99%');
  assert.equal(formatScalar(100, 'progress'), '100%');

  assert.equal(formatRecordedPercent(0), '0%');
  assert.equal(formatRecordedPercent(75), '75%');
  assert.equal(formatRecordedPercent(75.3), '75.3%');
  assert.equal(formatRecordedPercent(75.25), '75.25%');
  assert.equal(formatRecordedPercent(0.14 * 100), '14%');
  assert.equal(formatRecordedPercent(0.29 * 100), '28.99999%');
  assert.equal(formatRecordedPercent(-5.2), '−5.2%');
  assert.equal(formatRecordedPercent(15.5, { signed: true }), '+15.5%');
  assert.equal(formatRecordedPercent(-10.2, { signed: true }), '−10.2%');
  assert.equal(formatRecordedPercent(0, { signed: true }), '0%');
  assert.equal(formatScalar(11 / 5975, 'share'), '0.2%');
  assert.equal(formatScalar(0, 'share'), '0%');
  assert.deepEqual(formatValueState(knownValue(0.24), 'share'), { kind: 'known', text: '24%', exact: '24%', spoken: '24%', audit: '24%' });

  assert.equal(formatShareRatio(14_500n, 100_000n), '15%');
  assert.equal(formatShareRatio(2_850n, 100_000n), '2.9%');
  assert.equal(formatShareRatio(0n, 100_000n), '0%');
  assert.equal(formatShareRatio(100_000n, 100_000n), '100%');
  assert.equal(formatShareRatio(1n, 100_000n), '<0.1%');
  assert.equal(formatShareRatio(99_999n, 100_000n), '99%');
  assert.equal(formatRational(327_680_000_000_327n, 32_768n, { maxDecimals: 2 }), '10,000,000,000');
  assert.equal(formatRational(1n, 32_768n, { maxDecimals: 2 }), '<0.01');
  assert.deepEqual(formatCompactRational(597_999n * 32_768n + 20_000n, 32_768n, { maxDecimals: 0 }), {
    text: '597K',
    exact: '597,999',
    compacted: true,
  });
  assert.throws(() => formatRational(1n, 0n), InvalidFamilyValueError);
  assert.throws(() => formatShareRatio(1n, 0n), InvalidFamilyValueError);
  assert.throws(() => formatShareRatio(-1n, 10n), InvalidFamilyValueError);
});

test('R2-K3: the tooltip and the audit carry the recorded figure, not the one shortened for the screen', () => {
  const tradeFlow: ValueFormat = { decimals: 2, maxDecimals: 1, signed: true };
  assert.equal(formatScalar(16n, tradeFlow), '+0.1');
  assert.deepEqual(valueStateProps(knownValue(16n), tradeFlow), { kind: 'known', text: '+0.16' });
  assert.deepEqual(valueStateProps(knownValue(32n), tradeFlow), { kind: 'known', text: '+0.32' });
  const shown = formatValueState(knownValue(16n), tradeFlow);
  assert.equal(shown.kind === 'known' ? shown.text : null, '+0.1');
  assert.equal(shown.audit, '+0.16');
  const compact = formatCompactScalar(123_456n, { decimals: 2, maxDecimals: 0 });
  assert.deepEqual(compact, { text: '1.23K', exact: '1,234.56', compacted: true });
  assert.equal(formatCompactScalar(1234.6, 'integer').exact, '1,234.6');
  assert.equal(formatCompactScalar(7_136, 'integer').exact, '7,136');
  const partial = formatValueState(partialValue(16n, 'One market not read'), tradeFlow);
  assert.equal(partial.kind === 'partial' ? partial.text : null, '+0.1');
  assert.equal(partial.audit, '+0.16, partial: One market not read');
});

test('R2-K4: a number is cut from its own round-trip spelling, never rounded to 15 digits first', () => {
  assert.equal(formatScalar(Number.MAX_SAFE_INTEGER), '9,007,199,254,740,991');
  assert.equal(formatScalar(0.09999999999999998, { decimals: 0, maxDecimals: 1 }), '<0.1');
  assert.equal(formatScalar(0.09999999999999998, 'decimal1'), '<0.1');
  assert.equal(formatScalar(2.55, { decimals: 0, maxDecimals: 1 }), '2.5');
  assert.equal(formatScalar(1.005, { decimals: 0, maxDecimals: 2 }), '1');
  assert.equal(formatScalar(1e-7, { decimals: 0, maxDecimals: 7, fixedDecimals: true }), '0.0000001');
  assert.equal(formatScalar(1e21), '1,000,000,000,000,000,000,000');
  assert.equal(formatRecordedPercent(0.09999999999999998), '0.09999%');
  assert.equal(formatShare(0.0009999999999999998), '<0.1%');
});

test('R2-K14, R2-K15: the clock is reached only through the guarded entry, and progress truncates its own decimal', () => {
  assert.equal('formatClockSeconds' in valueFormat, false);
  assert.throws(() => formatScalar(null as unknown as number, 'clockSeconds'), InvalidFamilyValueError);
  assert.throws(() => formatScalar(Number.NaN, 'clockSeconds'), InvalidFamilyValueError);
  assert.equal(formatScalar(3_661, 'clockSeconds'), '1:01:01');
  assert.equal(formatScalar(3_661.9, 'clockSeconds'), '1:01:01');
  assert.equal(formatProgress(0.009999999999999), '0%');
  assert.equal(formatProgress(0.57), '57%');
  assert.equal(formatProgress(0.29), '29%');
  assert.equal(formatProgress(0.999999999999999), '99%');
});
