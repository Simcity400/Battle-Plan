import assert from 'node:assert/strict';
import test from 'node:test';
import { sortRows } from '../../charts/table/logic.ts';
import { liveKey } from '../../charts/live.ts';
import { countryMatch, membershipUnrecordedAt } from '../../dashboard/groups.ts';
import type { CountryGroupReply, NextElectionsReply } from '../../generated/dashboard';
import { countdownTexts, daysLeft, daysText, electionValues, electionsAt, electionsState, electionsStructure, electionTooltip, electionWarnings, hoursLeftAt, intervalText, termAt, termSortValues, termText, validateElectionsReply } from './nextElectionsData.ts';
import type { ElectionRow } from './nextElectionsData.ts';

const entry = (tag: string, fields: { key?: string; interval?: number | null; next?: number | null; last?: number | null } = {}) => ({
  key: fields.key ?? tag,
  tag,
  interval: fields.interval ?? null,
  next: fields.next ?? null,
  last: fields.last ?? null,
});
const reply = (rows: ReturnType<typeof entry>[], unknown = 0, recorded = true): NextElectionsReply => ({
  branchId: '1', recorded, rows, unknown, warnings: [],
});
const all = () => true;
const keyOf = (row: ElectionRow) => row.key;

test('days left and the term meter come from the recorded interval, next date and term start', () => {
  const data = validateElectionsReply(reply([entry('MOD', { interval: 4, next: 3000, last: 100 })]));
  const [row] = electionsAt(data, 1550, all).rows;
  assert.equal(row.tag, 'MOD');
  assert.equal(hoursLeftAt(row, 1550), 1450);
  assert.equal(termAt(row, 1550), 0.5);
  assert.equal(daysText(hoursLeftAt(row, 1550)!), '61 days');
  assert.equal(daysText(24), '1 day');
  assert.equal(daysLeft(1), 1);
  assert.equal(daysLeft(0), 0);
  assert.equal(termText(termAt(row, 1550)!), '50%');
  assert.equal(intervalText(48), 'Every 48 months');
  assert.equal(intervalText(1), 'Every 1 month');
  const tip = electionTooltip(row, { name: 'Moddia' }, 1550);
  assert.equal(tip.title, 'Moddia');
  assert.deepEqual(tip.rows.map(item => [item.label, item.value]), [
    ['Election interval', 'Every 4 months'], ['Next election', tip.rows[1].value], ['Days left', '61 days']]);
});

test('missing operands give null values, an empty meter and sort last in both directions', () => {
  const data = validateElectionsReply(reply([
    entry('ONE', { interval: 48, next: 90, last: 10 }),
    entry('TWO', { interval: 48 }),
    entry('THR', { next: 70, last: 0 }),
    entry('FOU', { interval: 12, next: 80 }),
  ]));
  const { rows } = electionsAt(data, 50, all);
  const byTag = new Map(rows.map(row => [row.tag, row]));
  assert.equal(byTag.get('TWO')!.next, null);
  assert.equal(hoursLeftAt(byTag.get('TWO')!, 50), null);
  assert.equal(termAt(byTag.get('TWO')!, 50), null);
  assert.equal(termAt(byTag.get('THR')!, 50), null);
  assert.equal(hoursLeftAt(byTag.get('THR')!, 50), 20);
  assert.equal(termAt(byTag.get('FOU')!, 50), null);
  assert.equal(termAt(byTag.get('ONE')!, 50), 0.5);
  const values = electionValues(50);
  for (const direction of ['ascending', 'descending'] as const) {
    assert.equal(sortRows(rows, values.days, direction, keyOf).at(-1)!.tag, 'TWO');
    assert.equal(sortRows(rows, values.next, direction, keyOf).at(-1)!.tag, 'TWO');
  }
  assert.deepEqual(sortRows(rows, values.days, 'ascending', keyOf).map(row => row.tag), ['THR', 'FOU', 'ONE', 'TWO']);
  const tip = electionTooltip(byTag.get('TWO')!, { name: 'Two' }, 50);
  assert.deepEqual(tip.rows.slice(1).map(item => item.value), ['Not recorded', 'Not recorded']);
});

test('unknown eligibility is surfaced as a warning and unavailable when no rows are shown', () => {
  const data = validateElectionsReply(reply([entry('YES', { next: 500 })], 4));
  const view = electionsAt(data, 50, all);
  assert.deepEqual(view.rows.map(row => row.tag), ['YES']);
  assert.equal(view.unknown, 4);
  assert.deepEqual(electionWarnings(data, view), ['4 refused election eligibility: not recorded at this hour']);
  const none = validateElectionsReply(reply([], 2));
  assert.deepEqual(electionsState(electionsAt(none, 50, all), null), { kind: 'unavailable', message: 'Election eligibility was not recorded at this date.' });
});

test('countdown texts and term order move with the shown hour without changing structure', () => {
  const data = validateElectionsReply(reply([
    entry('A', { interval: 48, next: 2400, last: 0 }),
    entry('B', { interval: 48, next: 1800, last: 600 }),
  ]));
  const rows = electionsAt(data, 600, all).rows;
  assert.equal(electionsStructure(data), electionsStructure(validateElectionsReply(reply([
    entry('A', { interval: 48, next: 2400, last: 0 }),
    entry('B', { interval: 48, next: 1800, last: 600 }),
  ]))));
  const at = countdownTexts(rows, 1200);
  assert.equal(at.get('days\u0000A'), '50 days');
  assert.equal(at.get('term\u0000A'), '50%');
  const values = termSortValues(rows);
  const order = (hour: number) => sortRows(rows, row => values(hour).get(liveKey(row.key, 'term')) ?? null, 'descending', keyOf).map(row => row.tag);
  assert.deepEqual(order(600), ['A', 'B']);
  assert.deepEqual(order(1500), ['B', 'A']);
});

test('a chosen group whose membership was not recorded at the hour is unavailable, never no elections', () => {
  const data = validateElectionsReply(reply([entry('MOD', { interval: 48, next: 300, last: 0 })]));
  const groups: CountryGroupReply = { branchId: '1', warnings: [], humans: [{ tag: 'MOD', firstHour: 0, lastHour: 130 }], factions: [], wars: [], warStarts: [],
    capitulations: [], identityChanges: [], countryIdentities: [{ tag: 'MOD', identities: [{ firstHour: 0, lastHour: 200, slot: 1, countryId: 10 }] }],
    unobserved: { humans: [{ firstHour: 110, lastHour: 120 }], factions: [], wars: [] } };
  const humans = { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false } as const;
  const match = countryMatch(humans, [], groups, 200);
  const at = (hour: number) => electionsState(electionsAt(data, hour, match.matches), membershipUnrecordedAt(humans, groups, hour));
  assert.deepEqual(at(115), { kind: 'unavailable', message: 'Group membership is unavailable at this time.' });
  assert.equal(at(100), null);
  assert.deepEqual(at(150), { kind: 'empty', message: 'No country holds elections at this date.' });
  assert.equal(membershipUnrecordedAt({ kind: 'all' }, groups, 115), null);
});

test('malformed at-hour replies are refused rather than read as no elections', () => {
  const good = () => reply([entry('MOD', { interval: 48, next: 20, last: 0 })]);
  for (const change of [
    (value: Record<string, unknown>) => { value.recorded = 'yes'; },
    (value: Record<string, unknown>) => { value.unknown = -1; },
    (value: Record<string, unknown>) => { (value.rows as Record<string, unknown>[])[0].key = ''; },
    (value: Record<string, unknown>) => { (value.rows as Record<string, unknown>[])[0].next = 1.5; },
    (value: Record<string, unknown>) => { delete (value.rows as Record<string, unknown>[])[0].last; },
  ]) { const value = good() as unknown as Record<string, unknown>; change(value); assert.throws(() => validateElectionsReply(value)); }
});

test('5,000 countries are listed without a cap', () => {
  const rows = Array.from({ length: 5000 }, (_, index) => entry(`C${index}`, { key: String(index), interval: 48, next: 2000 + index, last: 0 }));
  const view = electionsAt(validateElectionsReply(reply(rows)), 500, all);
  assert.equal(view.rows.length, 5000);
  assert.equal(new Set(view.rows.map(row => row.key)).size, 5000);
  assert.equal(sortRows(view.rows, electionValues(500).days, 'ascending', keyOf)[0].tag, 'C0');
});
