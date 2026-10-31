import assert from 'node:assert/strict';
import test from 'node:test';
import { sortRows } from '../../charts/table/logic.ts';
import { preparedThroughHour } from '../../dashboard/politicsHistory.ts';
import { daysLeft, daysText, ELECTION_VALUES, electionsAt, electionTooltip, electionWarnings, intervalText, termText, validateElectionsReply } from './nextElectionsData.ts';
import type { ElectionRow } from './nextElectionsData.ts';

const run = (firstHour: number, lastHour: number, value: string) => ({ firstHour, lastHour, value });
const holder = (tag: string, fields: { held?: ReturnType<typeof run>[]; refused?: ReturnType<typeof run>[]; interval?: ReturnType<typeof run>[];
  next?: ReturnType<typeof run>[]; last?: ReturnType<typeof run>[] }, tags = [{ hour: 0, tag }]) =>
  ({ tags, held: fields.held ?? [], refused: fields.refused ?? [], interval: fields.interval ?? [], next: fields.next ?? [], last: fields.last ?? [] });
const reply = (elections: ReturnType<typeof holder>[], lastHour: number | null = 10_000) => ({ branchId: '1', mode: 'elections', lastHour, tag: null, hour: null,
  known: [{ firstHour: 0, lastHour: 10_000 }], elections, names: {}, exileHistory: [], holders: {}, holderBalances: [], warnings: [] as string[] });
const all = () => true;
const keyOf = (row: ElectionRow) => row.key;

test('days left and the term meter come from the recorded interval, next date and term start', () => {
  // A modded four-month term: started at hour 100, next election at 3,000.
  const data = validateElectionsReply(reply([holder('MOD', { held: [run(0, 5000, '1')], interval: [run(0, 5000, '4')], next: [run(0, 5000, '3000')], last: [run(0, 5000, '100')] })]));
  const [row] = electionsAt(data, 1550, all).rows;
  assert.equal(row.tag, 'MOD');
  assert.equal(row.hoursLeft, 1450);
  assert.equal(row.term, 0.5);
  assert.equal(daysText(row.hoursLeft!), '61 days');
  assert.equal(daysText(24), '1 day');
  assert.equal(daysLeft(1), 1);
  assert.equal(daysLeft(0), 0);
  assert.equal(termText(row.term!), '50%');
  assert.equal(intervalText(48), 'Every 48 months');
  assert.equal(intervalText(1), 'Every 1 month');
  const tip = electionTooltip(row, { name: 'Moddia' });
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
  assert.equal(byTag.get('TWO')!.hoursLeft, null);
  assert.equal(byTag.get('TWO')!.term, null);
  // No interval, or no term start: no meter, though the next date stays known.
  assert.equal(byTag.get('THR')!.term, null);
  assert.equal(byTag.get('THR')!.hoursLeft, 20);
  assert.equal(byTag.get('FOU')!.term, null);
  assert.equal(byTag.get('ONE')!.term, 0.5);
  for (const direction of ['ascending', 'descending'] as const) {
    assert.equal(sortRows(rows, ELECTION_VALUES.days, direction, keyOf).at(-1)!.tag, 'TWO');
    assert.equal(sortRows(rows, ELECTION_VALUES.next, direction, keyOf).at(-1)!.tag, 'TWO');
  }
  assert.deepEqual(sortRows(rows, ELECTION_VALUES.days, 'ascending', keyOf).map(row => row.tag), ['THR', 'FOU', 'ONE', 'TWO']);
  const tip = electionTooltip(byTag.get('TWO')!, { name: 'Two' });
  assert.deepEqual(tip.rows.slice(1).map(item => item.value), ['Not recorded', 'Not recorded']);
});

test('after an election the next date takes over and the term starts again', () => {
  const data = validateElectionsReply(reply([holder('MOD', { held: [run(0, 400, '1')], interval: [run(0, 400, '1')],
    next: [run(0, 199, '200'), run(200, 400, '920')], last: [run(0, 199, '0'), run(200, 400, '200')] })]));
  assert.equal(electionsAt(data, 199, all).rows[0].next, 200);
  const after = electionsAt(data, 200, all).rows[0];
  assert.equal(after.next, 920);
  assert.equal(after.term, 0);
  assert.equal(after.hoursLeft, 720);
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

test('hours past the prepared history are pending, never empty', () => {
  const data = validateElectionsReply(reply([], 100));
  assert.equal(preparedThroughHour(data, 100), true);
  assert.equal(preparedThroughHour(data, 101), false);
  assert.equal(preparedThroughHour(validateElectionsReply(reply([], null)), 0), false);
  const pending = validateElectionsReply({ ...reply([], 200), coverage: { firstHour: 0, lastHour: 200, complete: false }, pending: [{ firstHour: 150, lastHour: 300 }] });
  assert.equal(preparedThroughHour(pending, 160), false);
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
    (value: ReturnType<typeof good>) => { value.mode = 'exiles'; },
  ]) { const value = good(); change(value); assert.throws(() => validateElectionsReply(value)); }
});

test('5,000 countries are listed without a cap', () => {
  const elections = Array.from({ length: 5000 }, (_, index) => holder(`C${index}`, { held: [run(0, 1000, '1')], interval: [run(0, 1000, '48')],
    next: [run(0, 1000, String(2000 + index))], last: [run(0, 1000, '0')] }));
  const view = electionsAt(validateElectionsReply(reply(elections)), 500, all);
  assert.equal(view.rows.length, 5000);
  assert.equal(new Set(view.rows.map(row => row.key)).size, 5000);
  assert.equal(sortRows(view.rows, ELECTION_VALUES.days, 'ascending', keyOf)[0].tag, 'C0');
});
