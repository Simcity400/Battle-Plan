import test from 'node:test';
import assert from 'node:assert/strict';
import { breakdownRows, formatPercent, linesPerMeasure, rawAt } from './stabilityData.ts';
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
  assert.equal(formatPercent('8950', 5), '8.9%');
  assert.equal(formatPercent('50', 5, true), '+<0.1%');
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

test('stability projection merges and deduplicates marker warnings with ranking warnings', async () => {
  const { stabilityProjection } = await import('./stabilityProjection.ts');
  const { createBoundedReadView } = await import('../../families/boundedView.ts');
  const shared = 'Not in rank order: Countries was not complete at this hour';
  const markerReason = 'Who holds slot 1 is not known: Countries was not complete at these hours (110–120)';
  const model = stabilityProjection.project(
    createBoundedReadView({
      hour: 200,
      panels: new Map<string, unknown>([
        ['rank', { hour: 200, refused: 'Countries was not complete at this hour', rows: [] }],
        ['markers', {
          hour: 200,
          nextChange: null,
          markers: [],
          warnings: [shared, markerReason, markerReason],
        }],
      ]),
    }),
    {
      tags: ['GER'],
      extent: [100, 200],
      showStability: true,
      showWarSupport: true,
      zoom: false,
      windowHours: null,
      gaps: [],
      surface: '#111111',
    },
  );
  assert.deepEqual(model.warnings, [shared, markerReason]);
});
