import assert from 'node:assert/strict';
import test from 'node:test';
import { sortRows } from '../../charts/table/logic.ts';
import { liveKey } from '../../charts/live.ts';
import { countryMatch, membershipUnrecordedAt } from '../../dashboard/groups.ts';
import type { CountryGroupReply } from '../../generated/dashboard';
import { changeHours, countdownTexts, daysLeft, daysText, electionValues, electionsAt, electionsState, electionTooltip, electionWarnings, hoursLeftAt, intervalText, settledHour, termAt, termSortValues, termText, validateElectionsReply } from './nextElectionsData.ts';
import type { ElectionRow } from './nextElectionsData.ts';

const run = (firstHour: number, lastHour: number, value: string) => ({ firstHour, lastHour, value });
const holder = (tag: string, fields: { held?: ReturnType<typeof run>[]; refused?: ReturnType<typeof run>[]; interval?: ReturnType<typeof run>[];
  next?: ReturnType<typeof run>[]; last?: ReturnType<typeof run>[] }, tags = [{ hour: 0, tag }]) =>
  ({ tags, held: fields.held ?? [], refused: fields.refused ?? [], interval: fields.interval ?? [], next: fields.next ?? [], last: fields.last ?? [] });
const reply = (elections: ReturnType<typeof holder>[], lastHour: number | null = 10_000) => ({ branchId: '1', mode: 'elections', lastHour, tag: null, hour: null,
  known: [{ firstHour: 0, lastHour: 10_000 }], elections, names: {}, holderBalances: [], warnings: [] as string[] });
const all = () => true;
const keyOf = (row: ElectionRow) => row.key;

test('days left and the term meter come from the recorded interval, next date and term start', () => {
  // A modded four-month term: started at hour 100, next election at 3,000.
  const data = validateElectionsReply(reply([holder('MOD', { held: [run(0, 5000, '1')], interval: [run(0, 5000, '4')], next: [run(0, 5000, '3000')], last: [run(0, 5000, '100')] })]));
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
    holder('ONE', { held: [run(0, 100, '1')], interval: [run(0, 100, '48')], next: [run(0, 100, '90')], last: [run(0, 100, '10')] }),
    holder('TWO', { held: [run(0, 100, '1')], interval: [run(0, 100, '48')] }),
    holder('THR', { held: [run(0, 100, '1')], next: [run(0, 100, '70')], last: [run(0, 100, '0')] }),
    holder('FOU', { held: [run(0, 100, '1')], interval: [run(0, 100, '12')], next: [run(0, 100, '80')] }),
  ]));
  const { rows } = electionsAt(data, 50, all);
  const byTag = new Map(rows.map(row => [row.tag, row]));
  assert.equal(byTag.get('TWO')!.next, null);
  assert.equal(hoursLeftAt(byTag.get('TWO')!, 50), null);
  assert.equal(termAt(byTag.get('TWO')!, 50), null);
  // No interval, or no term start: no meter, though the next date stays known.
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

test('after an election the next date takes over and the term starts again', () => {
  const data = validateElectionsReply(reply([holder('MOD', { held: [run(0, 400, '1')], interval: [run(0, 400, '1')],
    next: [run(0, 199, '200'), run(200, 400, '920')], last: [run(0, 199, '0'), run(200, 400, '200')] })]));
  assert.equal(electionsAt(data, 199, all).rows[0].next, 200);
  const after = electionsAt(data, 200, all).rows[0];
  assert.equal(after.next, 920);
  assert.equal(termAt(after, 200), 0);
  assert.equal(hoursLeftAt(after, 200), 720);
});

test('countries that do not hold elections are hidden, and unknown eligibility is counted, not listed', () => {
  const data = validateElectionsReply(reply([
    holder('YES', { held: [run(0, 100, '1')], next: [run(0, 100, '500')] }),
    holder('NOT', { held: [run(0, 100, '0')], interval: [run(0, 100, '48')], next: [run(0, 100, '500')] }),
    // Absent from the complete read at hour 50: it held no schedule then.
    holder('GAP', { held: [run(0, 40, '1'), run(60, 100, '1')] }),
    // Its schedule was read, its eligibility not.
    holder('SCH', { refused: [run(0, 100, '1')], interval: [run(0, 100, '48')] }),
    // Refused before its first decided read, after its last, and throughout.
    holder('LED', { refused: [run(0, 60, '1')], held: [run(61, 100, '1')] }),
    holder('TRL', { held: [run(0, 40, '1')], refused: [run(41, 100, '1')] }),
    holder('ALL', { refused: [run(0, 100, '1')], last: [run(0, 100, '0')] }),
    // Not yet read: neither listed nor counted.
    holder('NEW', { held: [run(80, 100, '1')] }),
  ]));
  const view = electionsAt(data, 50, all);
  assert.deepEqual(view.rows.map(row => row.tag), ['YES']);
  assert.equal(view.unknown, 4);
  assert.deepEqual(electionWarnings(data, view), ['4 refused election eligibility: not recorded at this hour']);
  // Once decided, a country is listed; the others stay unknown.
  const later = electionsAt(data, 90, all);
  assert.deepEqual(later.rows.map(row => row.tag), ['YES', 'GAP', 'LED', 'NEW']);
  assert.equal(later.unknown, 3);
  assert.equal(electionsAt(data, 50, tag => tag === 'ALL').unknown, 1);
  assert.deepEqual(electionWarnings(data, electionsAt(data, 101, all)), []);
});

test('rows are named by the holder tag at the hour and narrowed by country', () => {
  const tags = [{ hour: 0, tag: 'OLD' }, { hour: 100, tag: 'NEW' }];
  const data = validateElectionsReply(reply([holder('OLD', { held: [run(0, 200, '1')] }, tags), holder('OTH', { held: [run(0, 200, '1')] })]));
  assert.deepEqual(electionsAt(data, 50, all).rows.map(row => row.tag), ['OLD', 'OTH']);
  assert.deepEqual(electionsAt(data, 150, all).rows.map(row => row.tag), ['NEW', 'OTH']);
  // The holder keeps one row identity across its tag change.
  assert.equal(electionsAt(data, 50, all).rows[0].key, electionsAt(data, 150, all).rows[0].key);
  const chosen = (tag: string, hour: number) => tag === (hour < 100 ? 'OLD' : 'NEW');
  assert.deepEqual(electionsAt(data, 50, chosen).rows.map(row => row.tag), ['OLD']);
  assert.deepEqual(electionsAt(data, 150, chosen).rows.map(row => row.tag), ['NEW']);
});

test('the table is drawn at the last hour a schedule changed, and only the countdown moves between', () => {
  const tags = [{ hour: 0, tag: 'OLD' }, { hour: 100, tag: 'NEW' }];
  const data = validateElectionsReply(reply([holder('OLD', { held: [run(0, 59, '1'), run(60, 200, '0')], interval: [run(0, 200, '4')],
    next: [run(0, 200, '3000')], last: [run(0, 200, '100')] }, tags)]));
  const hours = changeHours(data);
  assert.deepEqual(hours, [0, 60, 100, 201, 10_001]);
  assert.equal(settledHour(hours, 59), 0);
  assert.equal(settledHour(hours, 60), 60);
  assert.equal(settledHour(hours, 150), 100);
  assert.equal(settledHour(hours, -5), -5, 'before any change the hour is its own');
  const rows = electionsAt(data, settledHour(hours, 30), all).rows;
  const shape = (hour: number) => electionsAt(data, hour, all).rows.map(row => [row.key, row.tag, row.interval, row.next, row.termStart]);
  assert.deepEqual(shape(30), shape(0), 'the same rows anywhere before the next change');
  const at = countdownTexts(rows, 1550);
  assert.equal(at.get(`days\u0000${rows[0].key}`), '61 days');
  assert.equal(at.get(`term\u0000${rows[0].key}`), '50%');
  // The meter, tooltip and sort read the shown hour, never the hour the rows were drawn at.
  assert.equal(termAt(rows[0], 1550), 0.5);
  assert.equal(electionTooltip(rows[0], { name: 'Old' }, 1550).rows[2].value, '61 days');
  // A country joining the chosen group at 40 changes the rows there, though no schedule does.
  assert.deepEqual(changeHours(data, [40, 60]), [0, 40, 60, 100, 201, 10_001]);
  const joined = (_tag: string, hour: number) => hour >= 40;
  assert.deepEqual(electionsAt(data, settledHour(changeHours(data, [40]), 45), joined).rows.map(row => row.tag), ['OLD']);
});

test('the term order the table sorts by moves with the shown hour', () => {
  // A's term runs 0 to 2,400 and B's 600 to 1,800: at 600 A has served more of its term, at 1,500 B has.
  const data = validateElectionsReply(reply([
    holder('A', { held: [run(0, 3000, '1')], interval: [run(0, 3000, '48')], next: [run(0, 3000, '2400')], last: [run(0, 3000, '0')] }),
    holder('B', { held: [run(0, 3000, '1')], interval: [run(0, 3000, '48')], next: [run(0, 3000, '1800')], last: [run(0, 3000, '600')] }),
  ]));
  const rows = electionsAt(data, 600, all).rows;
  const values = termSortValues(rows);
  const order = (hour: number) => sortRows(rows, row => values(hour).get(liveKey(row.key, 'term')) ?? null, 'descending', keyOf).map(row => row.tag);
  assert.deepEqual(order(600), ['A', 'B']);
  assert.deepEqual(order(1500), ['B', 'A'], 'the same rows, ordered at the hour shown');
});

test('a chosen group whose membership was not recorded at the hour is unavailable, never no elections', () => {
  const data = validateElectionsReply(reply([holder('MOD', { held: [run(0, 200, '1')], interval: [run(0, 200, '48')], next: [run(0, 200, '300')], last: [run(0, 200, '0')] })]));
  const groups: CountryGroupReply = { branchId: '1', warnings: [], humans: [{ tag: 'MOD', firstHour: 0, lastHour: 130 }], factions: [], wars: [], warStarts: [],
    capitulations: [], identityChanges: [], countryIdentities: [{ tag: 'MOD', identities: [{ firstHour: 0, lastHour: 200, slot: 1, countryId: 10 }] }],
    unobserved: { humans: [{ firstHour: 110, lastHour: 120 }], factions: [], wars: [] } };
  const humans = { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false } as const;
  const match = countryMatch(humans, [], groups, 200);
  const at = (hour: number) => electionsState(electionsAt(data, hour, match.matches), membershipUnrecordedAt(humans, groups, hour));
  assert.deepEqual(at(115), { kind: 'unavailable', message: 'Group membership is unavailable at this time.' });
  assert.equal(at(100), null, 'a recorded member is listed');
  assert.deepEqual(at(150), { kind: 'empty', message: 'No country holds elections at this date.' }, 'a recorded group without members holds none');
  assert.equal(membershipUnrecordedAt({ kind: 'all' }, groups, 115), null, 'every country needs no group history');
});

test('malformed histories are refused rather than read as no elections', () => {
  const good = () => reply([holder('MOD', { held: [run(0, 10, '1')], next: [run(0, 10, '20')], last: [run(0, 10, '0')] })]);
  for (const change of [
    (value: ReturnType<typeof good>) => { value.elections[0].held[0].value = '2'; },
    (value: ReturnType<typeof good>) => { value.elections[0].refused = [run(11, 12, '0')]; },
    (value: ReturnType<typeof good>) => { delete (value.elections[0] as Record<string, unknown>).refused; },
    (value: ReturnType<typeof good>) => { value.elections[0].next = [run(5, 10, '1'), run(0, 4, '2')]; },
    (value: ReturnType<typeof good>) => { value.elections[0].last[0].value = '1.5'; },
    (value: ReturnType<typeof good>) => { delete (value.elections[0] as Record<string, unknown>).last; },
    (value: ReturnType<typeof good>) => { value.elections[0].tags = [{ hour: 5, tag: 'A' }, { hour: 5, tag: 'B' }]; },
    (value: ReturnType<typeof good>) => { value.mode = 'country'; },
  ]) { const value = good(); change(value); assert.throws(() => validateElectionsReply(value)); }
});

test('5,000 countries are listed without a cap', () => {
  const elections = Array.from({ length: 5000 }, (_, index) => holder(`C${index}`, { held: [run(0, 1000, '1')], interval: [run(0, 1000, '48')],
    next: [run(0, 1000, String(2000 + index))], last: [run(0, 1000, '0')] }));
  const view = electionsAt(validateElectionsReply(reply(elections)), 500, all);
  assert.equal(view.rows.length, 5000);
  assert.equal(new Set(view.rows.map(row => row.key)).size, 5000);
  assert.equal(sortRows(view.rows, electionValues(500).days, 'ascending', keyOf)[0].tag, 'C0');
});
