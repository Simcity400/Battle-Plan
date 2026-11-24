import test from 'node:test';
import assert from 'node:assert/strict';
import { breakdownRows, formatPercent, linesPerMeasure, percentRuns, rawAt, readHourOf, stabilityEnd, zoomRange } from './stabilityData.ts';
import type { ChartBucket, ChartSummary } from '../../charts/chartSummary';
import { resolveCountries } from '../../dashboard/groups.ts';
import { definition } from './definition.ts';
import type { BreakdownLine, StabilityBreakdown } from '../../generated/dashboard';

const line = (position: number, kind: string, valueRaw: string, extra: Partial<BreakdownLine> = {}): BreakdownLine => ({
  line: position, kind, identity: '', identityId: 0, secondaryRaw: '0', valueRaw, text: null, label: null, group: null, groupKind: null, ...extra });

const GER: StabilityBreakdown = {
  hour: 100, readHour: 100, language: 'l_english', linesAvailable: true,
  stability: { valueRaw: '96000', clampRaw: '0', terms: [
    { term: 'base', valueRaw: '85000' }, { term: 'war', valueRaw: '0' }, { term: 'popularity', valueRaw: '6000' },
    { term: 'propaganda', valueRaw: '0' }, { term: 'stabilityFactor', valueRaw: '5000' }, { term: 'coastalProtection', valueRaw: '0' }] },
  warSupport: { valueRaw: '75000', clampRaw: '0', terms: [
    { term: 'base', valueRaw: '30000' }, { term: 'offensiveWar', valueRaw: '0' }, { term: 'defensiveWar', valueRaw: '0' },
    { term: 'tension', valueRaw: '40000' }, { term: 'bombing', valueRaw: '0' }, { term: 'interception', valueRaw: '0' },
    { term: 'heroes', valueRaw: '0' }, { term: 'convoys', valueRaw: '0' }, { term: 'propaganda', valueRaw: '0' },
    { term: 'warSupportFactor', valueRaw: '5000' }] },
  maxCommandPower: { valueRaw: '8000000', clampRaw: '0', terms: [] },
  modifiers: [
    { key: 'stability_factor', index: 141, good: true, totalRaw: '5000', remainderRaw: null,
      lines: [line(29, 'unpaired', '5000', { text: 'The Reichstag' })] },
    { key: 'war_support_factor', index: 149, good: true, totalRaw: '5000', remainderRaw: null,
      lines: [line(43, 'static', '5000', { identity: 'pride_of_the_fleet_country', label: 'Has Pride of The Fleet' })] },
  ],
};

test('stability and war support offer every chosen country without a widget limit', () => {
  const tags = Array.from({ length: 5000 }, (_, index) => `MOD ${index}`);
  const resolved = resolveCountries({ kind: 'tags', tags }, [], undefined, 100, definition.countries.max);
  assert.deepEqual(resolved.tags, tags);
  assert.equal(resolved.dropped, 0);
});

test('values are exact percent with two decimals, signed for parts', () => {
  assert.equal(formatPercent('96000', 5), '96.0%');
  assert.equal(formatPercent('5000', 5, true), '+5.0%');
  assert.equal(formatPercent('-8938', 5, true), '−8.9%');
  assert.deepEqual(percentRuns([{ firstHour: 1, lastHour: 2, valueRaw: '96000', startsSegment: true }], 5), [{ firstHour: 1, lastHour: 2, value: 96, breakBefore: true }]);
  assert.equal(rawAt([{ firstHour: 1, lastHour: 2, valueRaw: '96000', startsSegment: true }], 3), null);
});

test('the breakdown lists the game tooltip lines, named in the viewer language or by the recorded text', () => {
  const stability = breakdownRows(GER, 'stability', 5, 'Stability');
  assert.deepEqual(stability.rows.map(row => [row.label, row.value]), [['Stability', undefined], ['Base value', '+85.0%'], ['Party popularity', '+6.0%'], ['The Reichstag', '+5.0%'], ['Total', '96.0%']]);
  assert.equal(stability.recorded, true);
  assert.equal(stability.rows[3].tone, 'muted');
  assert.ok(stability.rows[0].strong && stability.rows[4].strong);
  const warSupport = breakdownRows(GER, 'warSupport', 5, 'War support');
  assert.deepEqual(warSupport.rows.map(row => [row.label, row.value]), [['War support', undefined], ['Base value', '+30.0%'], ['World tension', '+40.0%'], ['Has Pride of The Fleet', '+5.0%'], ['Total', '75.0%']]);
  assert.equal(warSupport.recorded, false);
});

test('a formula term this list does not name retains exact facts with its key in hover details', () => {
  const extra: StabilityBreakdown = { ...GER, stability: { ...GER.stability, terms: [...GER.stability.terms, { term: 'futureTerm', valueRaw: '2000' }, { term: 'missingTerm', valueRaw: null }, { term: 'zeroTerm', valueRaw: '0' }] } };
  const { rows, unavailable } = breakdownRows(extra, 'stability', 5, 'Stability');
  assert.deepEqual(rows.slice(-4, -1), [
    { label: 'Formula term name unavailable', lookupKey: 'futureTerm', value: '2000' },
    { label: 'Formula term name unavailable', lookupKey: 'missingTerm', value: 'Not recorded', tone: 'muted' },
    { label: 'Formula term name unavailable', lookupKey: 'zeroTerm', value: '0' },
  ]);
  assert.equal(unavailable, true);
});

test('lines that do not add up show the rest as its own part, and missing inputs stay missing', () => {
  const partial: StabilityBreakdown = { ...GER, modifiers: [{ ...GER.modifiers[0], totalRaw: '8000', remainderRaw: '3000' }],
    stability: { ...GER.stability, terms: GER.stability.terms.map(term => term.term === 'popularity' ? { ...term, valueRaw: null } : term) } };
  const { rows, unavailable } = breakdownRows(partial, 'stability', 5, 'Stability');
  assert.ok(rows.some(row => row.label === 'Other' && row.value === '+3.0%'));
  assert.ok(rows.some(row => row.label === 'Party popularity' && row.value === 'Not recorded'));
  assert.equal(unavailable, true);
  const stale = breakdownRows({ ...GER, linesAvailable: false }, 'stability', 5, 'Stability');
  assert.deepEqual(stale.rows.at(-2), { label: 'Modifiers', value: '+5.0%', tone: 'positive' });
});


test('a long breakdown lists its largest lines and counts the rest, so it fits the window', () => {
  const cap = linesPerMeasure(2);
  assert.equal(cap, 4);
  const lines = Array.from({ length: cap + 3 }, (_, index) => line(index, 'idea', String((index + 1) * 100), { label: `Spirit ${index}` }));
  const many: StabilityBreakdown = { ...GER, modifiers: [{ ...GER.modifiers[0], lines }] };
  const { rows } = breakdownRows(many, 'stability', 5, 'Stability', cap);
  assert.equal(rows.filter(row => row.label.startsWith('Spirit')).length, cap);
  assert.deepEqual(rows.find(row => row.label === '3 more lines'), { label: '3 more lines', value: '+0.6%', tone: 'muted' });
});

test('zoom fits both bounds of the values in view, even beyond 0-100% in a mod', () => {
  assert.deepEqual(zoomRange([90, 96]), [89, 97]);
  assert.deepEqual(zoomRange([120, 150]), [117, 153]);
  assert.equal(zoomRange(null), null);
});


test('an hour between reads of the inputs carries the read before it; before the first read nothing is known', () => {
  const reads = [{ firstHour: 100, lastHour: 100 }, { firstHour: 124, lastHour: 130 }];
  assert.equal(readHourOf(reads, 100), 100);
  assert.equal(readHourOf(reads, 110), 100);
  assert.equal(readHourOf(reads, 127), 127);
  assert.equal(readHourOf(reads, 200), 130);
  assert.equal(readHourOf(reads, 99), null);
});

test('an end label reads the value where the line ends, never that value taken for an hour', () => {
  const bucket = (firstHour: number, value: number): ChartBucket => ({ firstHour, lastHour: firstHour + 23, lastValueHour: firstHour + 23, minHour: firstHour,
    maxHour: firstHour, firstValue: value, lastValue: value, minValue: value, maxValue: value, firstBreak: false, hasBreak: false });
  const summary: ChartSummary = { from: 5000, to: 5047, width: 2, buckets: [bucket(5000, 0.452), bucket(5024, 0.613)] };
  assert.deepEqual(stabilityEnd(summary, 5047, 5), { value: 0.613, raw: '61300' });
  assert.equal(formatPercent(stabilityEnd(summary, 5047, 5)!.raw, 5), formatPercent('61300', 5));
  assert.deepEqual(stabilityEnd(summary, 5010, 5), { value: 0.452, raw: '45200' }, 'the last bin begun by the end');
  assert.equal(stabilityEnd(summary, 4000, 5), null, 'before the line nothing is labelled');
  assert.equal(stabilityEnd(undefined, 5047, 5), null);
});
