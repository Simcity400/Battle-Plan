import assert from 'node:assert/strict';
import test from 'node:test';
import { dayLabel } from '../../charts/draw.ts';
import { sortRows } from '../../charts/table/logic.ts';
import { liveKey, valuesOf } from '../../charts/live.ts';
import {
  balanceAbsence, balanceAxis, balanceRows, balanceStatusAt, balanceStructure, chooseBalance, countryHolder, driftParts, holderRecord, lifetimeSpans,
  balanceTableValues, memberSince, nameWarnings, plotRuns, recordFor, recordValue, rowHolder, statusText, validateBalanceList, validateCountryBalances,
  validateMemberships, weeklyState,
} from './balanceOfPowerData.ts';
import type { BalanceRow } from './balanceOfPowerData.ts';
import type { BalanceHistory, BalanceRecord } from '../../generated/dashboard';
import { countryMatch, membershipUnrecordedAt } from '../../dashboard/groups.ts';
import type { CountryGroupReply } from '../../generated/dashboard';

const run = (firstHour: number, lastHour: number, value: string) => ({ firstHour, lastHour, value });
const balance = (token: string, values: ReturnType<typeof run>[], extra: Partial<BalanceHistory> = {}): BalanceHistory => ({
  token, name: null, left: { token: `${token}_left`, name: null }, right: { token: `${token}_right`, name: null },
  ranges: [
    { token: `${token}_low`, name: 'Low', min: '-100000', max: '-25000' },
    { token: `${token}_high`, name: 'High', min: '25000', max: '100000' },
  ],
  values, record: values.map(item => run(item.firstHour, item.lastHour, '0')),
  range: values.map(item => run(item.firstHour, item.lastHour, BigInt(item.value) > 25000n ? `${token}_high` : BigInt(item.value) < -25000n ? `${token}_low` : `${token}_mid`)),
  trending: values.map(item => run(item.firstHour, item.lastHour, `${token}_right`)), ...extra,
});
const history = (fields: Record<string, unknown>, mode = 'country') => ({ branchId: '1', mode, lastHour: 10_000, tag: mode === 'country' ? 'MOD' : null,
  known: [{ firstHour: 0, lastHour: 10_000 }], elections: [], names: {}, holderBalances: [], warnings: [] as string[], ...fields });
const holder = (id: string, tags: [number, string][], balances: BalanceHistory[], shown: ReturnType<typeof run>[] = []) =>
  ({ holder: id, tags: tags.map(([hour, tag]) => ({ hour, tag })), balances, shown });
const record = (ordinal: number, token: string, members: (string | null)[], extra: Partial<BalanceRecord> = {}): BalanceRecord => ({
  ordinal, token, name: `${token} name`, left: { token: `${token}_left`, name: 'Left' }, right: { token: `${token}_right`, name: 'Right' },
  trending: { token: `${token}_right`, name: 'Right' }, range: { token: `${token}_high`, name: 'High', min: '25000', max: '100000' }, ranges: [],
  members: members.map(tag => ({ tag, daily: '10', weekly: '70' })), sources: [{ token: 'mod_source', name: 'Source', daily: '5', weekly: '35' }], ...extra,
});
const detail = (balances: BalanceRecord[], extra: Record<string, unknown> = {}) =>
  ({ branchId: '1', view: 'balances', exiles: [], balances, current: [{ tag: 'MOD', record: 0 }], unavailable: [], warnings: [] as string[], ...extra });
const listAt = (hour: number, value: unknown) => ({ ...validateBalanceList(value), hour });
const values = (byOrdinal: Record<number, number>) => valuesOf(new Map(Object.entries(byOrdinal).map(([ordinal, value]) => [liveKey(ordinal, 'value'), value])),
  'Not recorded at this hour');
const all = () => true;

test('a country history holds each holder its tag held, and the view follows the one holding it then', () => {
  const data = validateCountryBalances(history({ holderBalances: [
    holder('1', [[0, 'MOD'], [500, 'NEW']], [balance('guild', [run(0, 900, '30000')])], [run(0, 900, 'guild')]),
    holder('2', [[600, 'MOD']], [balance('court', [run(600, 900, '-5000')])], [run(600, 900, 'court')]),
  ] }));
  assert.equal(countryHolder(data, 'MOD', 100)!.holder, '1');
  assert.equal(countryHolder(data, 'MOD', 700)!.holder, '2', 'a reused tag is another holder');
  assert.equal(countryHolder(data, 'MOD', 550)!.holder, '1', 'between holders, the one that held it last');
  assert.throws(() => validateCountryBalances(history({ holderBalances: [holder('1', [[0, 'MOD']], [balance('a', [run(0, 10, '1.5')])])] })));
  assert.throws(() => validateCountryBalances(history({ holderBalances: [holder('1', [[0, 'MOD']], [balance('a', [run(5, 10, '1')]), balance('a', [run(0, 1, '1')])])] })));
  assert.throws(() => validateCountryBalances(history({}, 'elections')));
});

test('the shown balance is the saved one, else the one the game shows, and its state names gaps, never zero', () => {
  const guild = balance('guild', [run(0, 99, '30000'), run(200, 400, '-30000')]);
  const court = balance('court', [run(50, 400, '1000')]);
  const subject = { balances: [guild, court], shown: [run(0, 99, 'guild'), run(100, 400, 'court')] };
  assert.equal(chooseBalance(subject, '', 10, 400).balance?.token, 'guild');
  assert.equal(chooseBalance(subject, '', 150, 400).balance?.token, 'court');
  assert.equal(chooseBalance(subject, 'guild', 150, 400).balance?.token, 'guild');
  assert.equal(chooseBalance(subject, 'gone', 150, 400).missing, 'gone');
  assert.deepEqual(lifetimeSpans(guild), [{ firstHour: 0, lastHour: 99 }, { firstHour: 200, lastHour: 400 }]);
  assert.deepEqual(plotRuns(guild).map(item => item.value), [0.3, -0.3]);
  const active = balanceStatusAt(guild, 10, 400, []);
  assert.deepEqual(active, { kind: 'active', value: '30000', range: 'guild_high', trending: 'guild_right' });
  assert.equal(statusText(active, guild, {}), 'High · 0.30 · toward Side name unavailable');
  assert.deepEqual(balanceStatusAt(guild, 250, 400, []), { kind: 'active', value: '-30000', range: 'guild_low', trending: 'guild_right' });
  assert.deepEqual(balanceStatusAt(guild, 150, 400, []), { kind: 'ended', hour: 100, by: false, range: 'guild_high' });
  const unread = [{ firstHour: 100, lastHour: 199 }];
  const status = balanceStatusAt(guild, 150, 400, unread);
  assert.deepEqual(status, { kind: 'last-recorded', hour: 99, range: 'guild_high' });
  assert.equal(statusText(status, guild, {}), `Last recorded ${dayLabel(99)} in High`);
  assert.throws(() => validateCountryBalances(history({ holderBalances: [holder('1', [[0, 'MOD']], [{ ...guild, range: undefined } as unknown as BalanceHistory])] })));
  assert.deepEqual(balanceAbsence([{ firstHour: 0, lastHour: 99 }], 150, 400, unread), { kind: 'unread' });
  assert.deepEqual(balanceAbsence([{ firstHour: 0, lastHour: 400 }], 150, 400, []), { kind: 'none', recorded: false });
  assert.deepEqual(balanceAxis(guild), { min: -1, max: 1, ticks: [-1, -0.25, 0.25, 1] });
});

test('the list is the hour\'s records, one row each, kept when a member is chosen, and renders only when they change', () => {
  const reply = listAt(120, detail([record(0, 'guild', ['MOD', 'OTH']), record(1, 'court', ['ANO', null])]));
  const view = balanceRows(reply, all);
  assert.deepEqual(view.rows.map(row => [row.token, row.members.map(member => member.tag)]), [['guild', ['MOD', 'OTH']], ['court', ['ANO']]]);
  assert.equal(view.untagged, 1);
  assert.deepEqual(balanceRows(reply, tag => tag === 'OTH').rows.map(row => row.token), ['guild']);
  assert.deepEqual(rowHolder(view.rows[0], 120, tag => tag === 'OTH'), { tag: 'OTH' });
  const live = values({ 0: -60000, 1: 1000 });
  const table = balanceTableValues(live);
  assert.equal(sortRows(view.rows, (row: BalanceRow) => table.get(liveKey(row.key, 'value')), 'descending', (row: BalanceRow) => row.key)[0].token, 'guild');
  assert.equal(recordValue(live, { ordinal: 0 }), -60000n, 'signed and exact');
  assert.equal(recordValue(values({ 1: 1000 }), { ordinal: 0 }), null);
  const same = validateBalanceList(detail([record(0, 'guild', ['MOD', 'OTH']), record(1, 'court', ['ANO', null])]));
  assert.equal(balanceStructure(same), balanceStructure(reply));
  const moved = validateBalanceList(detail([record(0, 'guild', ['MOD', 'OTH'], { range: null }), record(1, 'court', ['ANO', null])]));
  assert.notEqual(balanceStructure(moved), balanceStructure(reply));
  assert.throws(() => validateBalanceList({ ...detail([]), view: 'exiles' }));
});

test('the weekly change reads the holder\'s record at the detail\'s hour, members and sources apart and exact', () => {
  const data = validateCountryBalances(history({ holderBalances: [holder('1', [[0, 'MOD']], [balance('guild', [run(0, 900, '30000')])], [run(0, 900, 'guild')])] }));
  const reply = listAt(100, detail([record(0, 'guild', ['MOD', 'OTH'])]));
  assert.equal(recordFor(reply, null, 'MOD')?.token, 'guild');
  assert.equal(holderRecord(reply, data.holderBalances[0], 'guild')?.ordinal, 0);
  const state = weeklyState(data, 'MOD', 100, reply, null);
  assert.equal(state.kind, 'record');
  const parts = driftParts((state as { record: BalanceRecord }).record, tag => ({ name: tag }));
  assert.equal(parts.weekly, 140n);
  assert.equal(parts.sourced, 35n);
  assert.deepEqual(parts.members.map(part => part.label), ['MOD', 'OTH']);
  const unread = listAt(100, detail([], { unavailable: [{ label: 'balances of power', reason: 'not read', count: 1 }] }));
  assert.equal(weeklyState(data, 'MOD', 100, unread, null).kind, 'unread');
  assert.deepEqual(nameWarnings([balance('guild', [])], {}, ['guild:guild_high']), ['1 balance name unavailable', '1 range name unavailable']);
  assert.deepEqual(nameWarnings([balance('guild', [run(0, 10, '0'), run(11, 20, '30000')])], {}, ['guild:guild_mid']),
    ['1 balance name unavailable', '1 range name unavailable']);
});

test('a row has been held since its first member was listed under its balance without a break, from the memberships alone', () => {
  const listed = (token: string, record: ReturnType<typeof run>[]): BalanceHistory => ({ ...balance(token, []), record });
  const data = validateMemberships(history({ holderBalances: [
    holder('1', [[0, 'MOD'], [500, 'NEW']], [listed('guild', [run(0, 99, '0'), run(100, 300, '2'), run(400, 900, '0')])]),
  ] }, 'memberships'));
  assert.equal(memberSince(data, 'MOD', 'guild', 250), 0, 'a record moving its stored position is no break');
  assert.equal(memberSince(data, 'NEW', 'guild', 600), 400, 'renamed, the same holder; a gap is a break');
  assert.equal(memberSince(data, 'MOD', 'guild', 350), null, 'not listed then');
  assert.equal(memberSince(data, 'MOD', 'court', 50), null);
  assert.throws(() => validateMemberships(history({}, 'country')));
});

const humans = { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false } as const;
const gapGroups = (tags: readonly string[], gap = { firstHour: 110, lastHour: 120 }): CountryGroupReply => ({ branchId: '1', warnings: [],
  humans: tags.map(tag => ({ tag, firstHour: 0, lastHour: 200 })), factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [],
  countryIdentities: tags.map((tag, slot) => ({ tag, identities: [{ firstHour: 0, lastHour: 200, slot, countryId: slot + 1 }] })),
  unobserved: { humans: [gap], factions: [], wars: [] } });
const GAP = 'Group membership is unavailable at this time.';

test('a balance left out where the chosen group’s membership was not recorded makes the hour unknown, never no balance', () => {
  const groups = gapGroups(['MOD', 'OTH']);
  const match = countryMatch(humans, [], groups, 200), unrecorded = (hour: number) => membershipUnrecordedAt(humans, groups, hour);
  const view = (hour: number) => balanceRows(listAt(hour, detail([record(0, 'guild', ['MOD', 'OTH'])])), match.matches, unrecorded);
  assert.deepEqual([view(115).rows.length, view(115).membership], [0, GAP]);
  assert.deepEqual([view(100).rows.map(row => row.token), view(100).membership], [['guild'], null]);
});
