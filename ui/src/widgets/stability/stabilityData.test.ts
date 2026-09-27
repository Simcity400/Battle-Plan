import test from 'node:test';
import assert from 'node:assert/strict';
import type { BreakdownLine, StabilityBreakdown } from '../../generated/dashboard';
import { breakdownRows, formatPercent, lastSampleHour, lineName, linesPerMeasure, percentRuns, rawAt, zoomRange } from './stabilityData.ts';

const line = (position: number, kind: string, valueRaw: string, extra: Partial<BreakdownLine> = {}): BreakdownLine => ({
  line: position, kind, identity: '', identityId: 0, secondaryRaw: '0', valueRaw, text: null, label: null, group: null, groupKind: null, ...extra });

// GER on 28 Oct 1943, as the game's tooltip showed it: Stability 85 + 6 + 5 = 96%,
// War Support 30 + 40 + 5 = 75%.
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

test('lines that do not add up show the rest as its own part, and missing inputs stay missing', () => {
  const partial: StabilityBreakdown = { ...GER, modifiers: [{ ...GER.modifiers[0], totalRaw: '8000', remainderRaw: '3000' }],
    stability: { ...GER.stability, terms: GER.stability.terms.map(term => term.term === 'popularity' ? { ...term, valueRaw: null } : term) } };
  const { rows, unavailable } = breakdownRows(partial, 'stability', 5, 'Stability');
  assert.ok(rows.some(row => row.label === 'Other / not identified' && row.value === '+3.0%'));
  assert.ok(rows.some(row => row.label === 'Party popularity' && row.value === 'Not recorded'));
  assert.equal(unavailable, true);
  // Lines from another read are not shown as this hour's: the term stands alone.
  const stale = breakdownRows({ ...GER, linesAvailable: false }, 'stability', 5, 'Stability');
  assert.deepEqual(stale.rows.at(-2), { label: 'Modifiers', value: '+5.0%', tone: 'positive' });
});

test('a line is named by its label with what it is, else its recorded text, else its identity', () => {
  assert.deepEqual(lineName(line(1, 'idea', '1', { identity: 'x', label: 'Name' })), { name: 'Name', recorded: false });
  assert.deepEqual(lineName(line(1, 'idea', '1', { label: 'Blackouts', groupKind: 'national_spirit' })), { name: 'National spirit: Blackouts', recorded: false });
  assert.deepEqual(lineName(line(1, 'advisor', '1', { label: 'Hjalmar Schacht', group: 'Political Advisor' })), { name: 'Political Advisor: Hjalmar Schacht', recorded: false });
  assert.deepEqual(lineName(line(1, 'countryLeader', '1', { label: 'Iosif Stalin' })), { name: 'Country leader: Iosif Stalin', recorded: false });
  assert.deepEqual(lineName(line(1, 'unpaired', '1', { text: 'Game text' })), { name: 'Game text', recorded: true });
  assert.deepEqual(lineName(line(1, 'idea', '1', { identity: 'x' })), { name: 'x', recorded: false });
});

test('a long breakdown lists its largest lines and counts the rest, so it fits the window', () => {
  const cap = linesPerMeasure(2);
  assert.equal(cap, 4);
  const lines = Array.from({ length: cap + 3 }, (_, index) => line(index, 'idea', String((index + 1) * 100), { label: `Spirit ${index}` }));
  const many: StabilityBreakdown = { ...GER, modifiers: [{ ...GER.modifiers[0], lines }] };
  const { rows } = breakdownRows(many, 'stability', 5, 'Stability', cap);
  assert.equal(rows.filter(row => row.label.startsWith('Spirit')).length, cap);
  // The three smallest (100 + 200 + 300) are counted in one row.
  assert.deepEqual(rows.find(row => row.label === '3 more lines'), { label: '3 more lines', value: '+0.6%', tone: 'muted' });
});

test('zoom fits both bounds of the values in view, even beyond 0-100% in a mod', () => {
  assert.deepEqual(zoomRange([90, 96]), [89, 97]);
  assert.deepEqual(zoomRange([120, 150]), [117, 153]);
  assert.equal(zoomRange(null), null);
});

test('a line ending early is labelled at its own last sample', () => {
  const run = (firstHour: number, lastHour: number) => ({ firstHour, lastHour, value: 50, breakBefore: false });
  const runs = [run(0, 99), run(150, 200)];
  assert.equal(lastSampleHour(runs, 300), 200);
  assert.equal(lastSampleHour(runs, 120), 99);
  assert.equal(lastSampleHour(runs, 160), 160);
  assert.equal(lastSampleHour([], 10), null);
  assert.equal(lastSampleHour(runs, -5), null);
});
