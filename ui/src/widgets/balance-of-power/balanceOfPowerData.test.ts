import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import type { ArtResult } from '../../families/contract.ts';
import type { BalanceHistory, BalanceRecord, PoliticsHistoryReply, PoliticsStateReply } from '../../generated/dashboard';
import { balanceAxis, balanceHistoryProof, recordAt, selectedBalance, validateBalanceList, validateCountryBalances } from './balanceOfPowerData.ts';
import { balanceOfPowerProjection } from './balanceOfPowerProjection.ts';

const run = (firstHour: number, lastHour: number, value: string) => ({ firstHour, lastHour, value });
const balance = (token: string, values: ReturnType<typeof run>[], extra: Partial<BalanceHistory> = {}): BalanceHistory => ({
  token,
  name: null,
  left: { token: `${token}_left`, name: null },
  right: { token: `${token}_right`, name: null },
  ranges: [
    { token: `${token}_low`, name: 'Low', min: '-100000', max: '-25000' },
    { token: `${token}_high`, name: 'High', min: '25000', max: '100000' },
  ],
  values,
  record: values.map(item => run(item.firstHour, item.lastHour, '0')),
  range: [],
  trending: [],
  ...extra,
});
const history = (hour: number, fields: Partial<PoliticsHistoryReply> = {}): PoliticsHistoryReply => ({
  branchId: '1',
  mode: 'country',
  lastHour: 10_000,
  hour,
  nextChange: null,
  unread: null,
  selected: null,
  tag: 'AAA',
  known: [{ firstHour: 0, lastHour: hour }],
  elections: [],
  names: {},
  holderBalances: [],
  warnings: [],
  ...fields,
});
const holder = (balances: BalanceHistory[]) => ({ holder: '1', tags: [{ hour: 0, tag: 'AAA' }], balances, shown: [] });
const record = (ordinal: number, token: string, extra: Partial<BalanceRecord> = {}): BalanceRecord => ({
  ordinal,
  token,
  name: 'Invented balance',
  left: { token: 'side_west', name: 'West Council' },
  right: { token: 'side_east', name: 'East Assembly' },
  trending: { token: 'side_east', name: 'East Assembly' },
  range: { token: 'range_high', name: 'Upper Hand', min: '25000', max: '100000' },
  ranges: [],
  members: [{ tag: 'AAA', daily: '1000', weekly: null }],
  change: { daily: '1000', weekly: '0', weeklyChange: '7000', shown: 'daily', rate: '1000' },
  sources: null,
  ...extra,
});
const detail = (balances: BalanceRecord[]): PoliticsStateReply => ({
  branchId: '1', view: 'balances', exiles: [], exilesMissing: null, balances, current: [{ tag: 'AAA', record: 0 }], unavailable: [], warnings: [],
});
const NO_ART: ArtResult = { status: 'noArtExpected' };
const options = { surface: '#111111', subject: '\u00007', extent: [0, 1000] as const, branchFirstHour: 0, branchGaps: [], gaps: [] };
const project = (hour: number, panels: [string, unknown][], hovers?: Map<string, { hour: number; data: unknown }>) => balanceOfPowerProjection.project(createBoundedReadView({
  hour, panels: new Map<string, unknown>(panels), ...(hovers ? { hovers } : {}), art: { resolve: () => NO_ART },
}), options);

test('the history is the product\'s answer at its hour: its balance, its proof and only one holder', () => {
  const reply = validateCountryBalances(history(800, { selected: 'bop_a', nextChange: 900, holderBalances: [holder([balance('bop_a', [run(500, 800, '36000')])])] }));
  assert.equal(selectedBalance(reply)?.token, 'bop_a');
  assert.deepEqual(balanceHistoryProof(reply, [0, 800], 850), [800, 899]);
  assert.equal(balanceHistoryProof(reply, [0, 800], 900), null);
  assert.throws(() => validateCountryBalances(history(800, { known: [{ firstHour: 0, lastHour: 801 }] })), 'nothing read after the hour');
  assert.throws(() => validateCountryBalances(history(800, { holderBalances: [holder([]), holder([])] })), 'one recorded slot, one holder');
  assert.deepEqual(balanceAxis(balance('bop_a', [])), { min: -1, max: 1, ticks: [-1, -0.25, 0.25, 1] });
});

test('a record is joined by the ordinal the history recorded at that hour, never by its token alone', () => {
  const bop = balance('bop_a', [run(0, 99, '1000'), run(100, 200, '2000')], { record: [run(0, 99, '0'), run(100, 200, '3')] });
  const state = validateBalanceList(detail([record(0, 'bop_a'), record(3, 'bop_a', { name: 'Later record' })]));
  assert.equal(recordAt(state, bop, 50)?.ordinal, 0);
  assert.equal(recordAt(state, bop, 150)?.name, 'Later record');
  assert.equal(recordAt(detail([record(5, 'bop_a')]), bop, 150), null, 'another record with the same balance is never taken');
});

test('the chart draws the answer, keeps an unnamed range and side by key with its reason, and audits what it shows', () => {
  const hist = history(800, { selected: 'bop_a', nextChange: 900, holderBalances: [holder([balance('bop_a', [run(500, 800, '36000')])])] });
  const named = project(820, [['history', hist], ['state', detail([record(0, 'bop_a')])]]);
  const line = named.series[0]!;
  assert.equal(line.points!.at(-1)!.hour, 820, 'the open run reaches the displayed hour inside the proof');
  assert.equal(line.endLabel?.name, 'Upper Hand');
  assert.equal(line.endLabel?.value, '0.36');
  assert.deepEqual(named.overlays?.map(item => item.text), ['East Assembly', 'West Council']);
  assert.deepEqual(named.gaps?.map(gap => [gap.firstHour, gap.lastHour]), [[0, 500]]);
  assert.equal(named.auditRows![0]!.value, '+0.36|Upper Hand|range_high|East Assembly|West Council');

  const unnamed = project(800, [['history', hist], ['state', detail([record(0, 'bop_a', {
    range: { token: 'range_high', name: null, reason: 'missing localisation', min: '25000', max: '100000' },
    left: { token: 'side_west', name: null, reason: 'characters table was not recorded' },
  })])]]);
  assert.equal(unnamed.series[0]!.endLabel?.name, 'Range name unavailable', 'an unresolved active range is said, never dropped');
  assert.equal(unnamed.overlays?.[1]?.text, 'side_west', 'an unresolved side shows its recorded key');
  assert.notEqual(unnamed.auditRows![0]!.value, named.auditRows![0]!.value, 'correcting the displayed range changes the audit');
});

test('a hover reads the record at the hit hour: unread is not "Not active", and the change is the product\'s', () => {
  const hist = history(800, { selected: 'bop_a', known: [{ firstHour: 0, lastHour: 299 }, { firstHour: 400, lastHour: 800 }],
    holderBalances: [holder([balance('bop_a', [run(500, 800, '36000')])])] });
  const hover = (hour: number, data: PoliticsStateReply) => project(800, [['history', hist], ['state', detail([record(0, 'bop_a')])]], new Map([['hover', { hour, data }]])).hoverDetail!;
  const unread = hover(350, detail([]));
  assert.equal(unread.items[0]!.missingReason, 'The balance of power was not recorded at this hour');
  assert.equal(hover(450, detail([])).items[0]!.value, 'Not active', 'read and not a member is not active');
  const at = hover(600, detail([record(0, 'bop_a', { change: { daily: '1000', weekly: '0', weeklyChange: '7000', shown: 'daily', rate: '1000' } })]));
  assert.ok(at.items.some(item => item.label === 'Daily change' && item.value === '+0.01'), 'a daily drift is never a zero weekly change');
  const other = hover(600, detail([record(9, 'bop_a', { range: { token: 'range_low', name: 'Other', min: null, max: null } })]));
  assert.ok(other.items.some(item => item.label === 'Range' && item.missingReason), 'another record of the same balance is never joined');
});

test('no balance is said with its reason: unread, unread before, or none by this date', () => {
  const none = (fields: Partial<PoliticsHistoryReply>, hour = 800) => project(hour, [['history', history(800, fields)]]).empty;
  assert.equal(none({ known: [] })?.kind, 'unavailable');
  assert.equal(none({ unread: 'The balance of power was not read for part of this campaign before this date' })?.kind, 'unavailable');
  assert.match(none({})?.message ?? '', /has no balance of power by /);
});
