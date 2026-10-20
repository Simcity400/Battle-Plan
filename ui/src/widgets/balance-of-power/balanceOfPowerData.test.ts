import assert from 'node:assert/strict';
import test from 'node:test';
import { dayLabel } from '../../charts/draw.ts';
import { sortRows } from '../../charts/table/logic.ts';
import { createWidget, resolveWindow } from '../../dashboard/model.ts';
import type { BoundTrack } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options.ts';
import { tableSortOf } from '../../charts/table/options.ts';
import type { BalanceHistory, BalanceRecord } from '../../generated/dashboard';
import {
  balanceAbsence, balanceAxis, balanceLanes, balanceRowsAt, balanceStatusAt, chooseBalance, countryHolder, detailWarnings, driftParts, exactValue, holderRecord,
  lifetimeSpans, nameWarnings, pendingSpans, pinBalanceDrill, plotRuns, recordFor, ROW_VALUES, rowHolder, statusText, validateBalanceDetails,
  validateCountryBalances, validateHolderBalances, weeklyState,
} from './balanceOfPowerData.ts';
import type { BalanceRow } from './balanceOfPowerData.ts';
import { DEFAULT_SORT, OPTIONS, singleCountry } from './definition.ts';
import { unknownHours, unreadSpans } from '../../dashboard/politicsHistory.ts';
import '../definitions.ts';

const run = (firstHour: number, lastHour: number, value: string) => ({ firstHour, lastHour, value });
/** A mod's own balance: every token and name here is the synthetic content's, none from vanilla. */
const balance = (token: string, values: ReturnType<typeof run>[], extra: Partial<BalanceHistory> = {}): BalanceHistory => ({
  token, name: null, left: { token: `${token}_left`, name: null }, right: { token: `${token}_right`, name: null },
  ranges: [
    { token: `${token}_low`, name: null, min: '-100000', max: '-25000' },
    { token: `${token}_mid`, name: null, min: '-25000', max: '25000' },
    { token: `${token}_high`, name: null, min: '25000', max: '100000' },
  ],
  values, record: values.map(item => run(item.firstHour, item.lastHour, '0')),
  range: values.map(item => run(item.firstHour, item.lastHour, BigInt(item.value) > 25000n ? `${token}_high` : BigInt(item.value) < -25000n ? `${token}_low` : `${token}_mid`)),
  trending: values.map(item => run(item.firstHour, item.lastHour, `${token}_right`)),
  ...extra,
});
const history = (fields: Record<string, unknown>, mode = 'country', lastHour: number | null = 10_000) => ({ branchId: '1', mode, lastHour, tag: mode === 'country' ? 'MOD' : null,
  known: [{ firstHour: 0, lastHour: lastHour ?? 0 }], elections: [], names: {}, exileHistory: [], holders: {}, holderBalances: [], warnings: [] as string[], ...fields });
const holder = (id: string, tags: [number, string][], balances: BalanceHistory[], shown: ReturnType<typeof run>[] = []) =>
  ({ holder: id, tags: tags.map(([hour, tag]) => ({ hour, tag })), balances, shown });
/** Read at every hour of the fixtures. */
const READ = [{ firstHour: 0, lastHour: 10_000 }];
const names = {
  guild_struggle: 'Guild Struggle', guild_struggle_low: 'Crafts Ascendant', guild_struggle_mid: 'Uneasy Truce', guild_struggle_high: 'Merchants Ascendant',
  guild_struggle_right: 'the Merchants', guild_struggle_left: 'the Crafts', court_feud: 'Court Feud',
};

test('validation admits the country and holder histories and the hour detail, and refuses malformed ones', () => {
  const country = validateCountryBalances(history({ holderBalances: [holder('1', [[0, 'MOD']], [balance('guild_struggle', [run(0, 10, '1000')])], [run(0, 10, 'guild_struggle')])], names }));
  assert.equal(country.holderBalances[0].balances[0].token, 'guild_struggle');
  const one = (...balances: BalanceHistory[]) => history({ holderBalances: [holder('1', [[0, 'MOD']], balances)] });
  assert.throws(() => validateCountryBalances(one(balance('a', [run(0, 10, '1.5')]))));
  assert.throws(() => validateCountryBalances(one(balance('a', [run(5, 10, '1')]), balance('a', [run(0, 1, '1')]))), /malformed/);
  assert.throws(() => validateCountryBalances(one(balance('a', [run(5, 10, '1'), run(8, 12, '2')]))));
  assert.throws(() => validateCountryBalances(history({}, 'balances')));
  assert.throws(() => validateCountryBalances({ ...history({}), known: [{ firstHour: 5, lastHour: 1 }] }), 'read coverage must be ordered spans');
  // A range whose authored bounds the content does not establish keeps its other facts.
  const unbounded = balance('a', [run(0, 10, '1')], { ranges: [{ token: 'a_low', name: null, min: null, max: null }] });
  assert.equal(validateCountryBalances(one(unbounded)).holderBalances[0].balances[0].ranges[0].min, null);
  assert.deepEqual(balanceAxis(unbounded), { min: -1, max: 1, ticks: undefined });
  const holders = validateHolderBalances(history({ holderBalances: [{ holder: '7', tags: [{ hour: 0, tag: 'MOD' }], balances: [], shown: [] }] }, 'balances'));
  assert.equal(holders.holderBalances.length, 1);
  assert.throws(() => validateHolderBalances(history({ holderBalances: [{ holder: '7', tags: [{ hour: 5, tag: 'A' }, { hour: 1, tag: 'B' }], balances: [], shown: [] }] }, 'balances')));
  const details = validateBalanceDetails({ branchId: '1', hour: 40, view: 'balances', exiles: [], balances: [], current: null, unavailable: [], warnings: [] }, 40);
  assert.equal(details.current, null);
  assert.throws(() => validateBalanceDetails({ branchId: '1', hour: 41, view: 'balances', exiles: [], balances: [], current: null, unavailable: [], warnings: [] }, 40));
});

test('the current balance is the one the shown run names at the hour; a saved one wins when the country has it', () => {
  const subject = { balances: [balance('court_feud', [run(0, 100, '0')]), balance('guild_struggle', [run(10, 100, '50000')])],
    shown: [run(0, 9, 'court_feud'), run(10, 100, 'guild_struggle')] };
  assert.equal(chooseBalance(subject, '', 5, 100).balance?.token, 'court_feud');
  assert.equal(chooseBalance(subject, '', 50, 100).balance?.token, 'guild_struggle', 'the game shows the first record listing the country, as the shown run says');
  const saved = chooseBalance(subject, 'court_feud', 50, 100);
  assert.equal(saved.balance?.token, 'court_feud');
  assert.equal(saved.automatic, false);
  const missing = chooseBalance(subject, 'gone_balance', 50, 100);
  assert.equal(missing.balance?.token, 'guild_struggle');
  assert.equal(missing.missing, 'gone_balance');
  // No shown run at the hour: the latest shown before it, else the first shown by the cut-off.
  const later = { balances: [balance('guild_struggle', [run(200, 300, '0')])], shown: [run(200, 300, 'guild_struggle')] };
  assert.equal(chooseBalance(later, '', 50, 1000).balance?.token, 'guild_struggle');
  assert.equal(chooseBalance(later, '', 50, 100).balance, null, 'Live hides a balance that starts after the playback date');
});

test('a country with two balances at once has a lane for each, and a shared record is one row listing every member', () => {
  const guild = balance('guild_struggle', [run(0, 100, '30000')]);
  const court = balance('court_feud', [run(20, 100, '-5000')]);
  const lanes = balanceLanes({ balances: [court, guild], shown: [run(0, 100, 'guild_struggle')] }, names, 100);
  assert.deepEqual(lanes.map(lane => [lane.token, lane.name]), [['guild_struggle', 'Guild Struggle'], ['court_feud', 'Court Feud']]);
  const second = (value: string, record: string) => balance('guild_struggle', [run(0, 100, value)], { record: [run(0, 100, record)] });
  const holders = [
    holder('1', [[0, 'AAA']], [guild, court]),
    holder('2', [[0, 'BBB']], [second('30000', '0')]),
    // Same identity, another record: another row.
    holder('3', [[0, 'CCC']], [second('-30000', '1')]),
    // Every field equal to the first record, but stored as its own record: never joined.
    holder('4', [[0, 'DDD']], [second('30000', '2')]),
  ];
  const view = balanceRowsAt({ holderBalances: holders, names, known: READ }, 50, () => true)!;
  assert.equal(view.rows.length, 4);
  assert.deepEqual(view.rows.find(row => row.members[0].tag === 'DDD')!.members.map(member => member.tag), ['DDD'], 'equal values are not shared membership');
  const shared = view.rows.find(row => row.token === 'guild_struggle' && row.value === '30000')!;
  assert.deepEqual(shared.members.map(member => member.tag), ['AAA', 'BBB']);
  assert.equal(shared.name, 'Guild Struggle');
  // Several chosen countries keep the rows listing any of them, with every member still named.
  const chosen = balanceRowsAt({ holderBalances: holders, names, known: READ }, 50, tag => tag === 'BBB')!;
  assert.deepEqual(chosen.rows.map(row => row.members.map(member => member.tag)), [['AAA', 'BBB']]);
  assert.equal(rowHolder(chosen.rows[0], 50, tag => tag === 'BBB').holder, '2', 'a row opens the chosen member');
  // Strongest lean first, by exact magnitude.
  const sort = tableSortOf(resolveOptions(OPTIONS).tableSort, DEFAULT_SORT);
  assert.deepEqual(sort, { column: 'value', direction: 'descending' });
  const sorted = sortRows(view.rows, ROW_VALUES.value, sort.direction, (row: BalanceRow) => row.key);
  assert.deepEqual(sorted.map(row => row.value.replace('-', '')), ['30000', '30000', '30000', '5000']);
});

test('lifetime lanes and the header follow the Live cut-off and the playback date', () => {
  const guild = balance('guild_struggle', [run(100, 199, '10000'), run(200, 400, '40000')]);
  const court = balance('court_feud', [run(300, 500, '0')]);
  const subject = { balances: [guild, court], shown: [run(100, 400, 'guild_struggle'), run(401, 500, 'court_feud')] };
  assert.deepEqual(lifetimeSpans(guild), [{ firstHour: 100, lastHour: 400 }], 'touching runs are one lifetime');
  const cut = balanceLanes(subject, names, 250);
  assert.deepEqual(cut.map(lane => lane.token), ['guild_struggle'], 'a balance starting after the playback date is not drawn yet');
  assert.deepEqual(cut[0].spans, [{ firstHour: 100, lastHour: 250 }]);
  assert.equal(cut[0].cut, true);
  assert.equal(balanceLanes(subject, names, 1000).length, 2);
  const none = () => false;
  assert.equal(statusText(balanceStatusAt(guild, 250, 250, [], none), guild, names), 'Merchants Ascendant · 0.40 · toward the Merchants');
  assert.equal(statusText(balanceStatusAt(court, 200, 1000, [], none), court, names), `Starts ${dayLabel(300)}`);
  assert.equal(statusText(balanceStatusAt(guild, 450, 1000, [], none), guild, names), `Ended ${dayLabel(401)} in Merchants Ascendant`);
});

test('hours not recorded stay unknown: the line breaks, nothing is carried and no end is claimed', () => {
  const guild = balance('guild_struggle', [run(0, 99, '20000'), run(150, 300, '-20000')]);
  const gap = [{ firstHour: 100, lastHour: 149 }];
  const runs = plotRuns(guild);
  assert.deepEqual(runs.map(item => [item.firstHour, item.lastHour, item.value]), [[0, 99, 0.2], [150, 300, -0.2]]);
  assert.deepEqual(lifetimeSpans(guild), [{ firstHour: 0, lastHour: 99 }, { firstHour: 150, lastHour: 300 }]);
  const unknown = unknownHours(gap, [], 0, null, []);
  const status = balanceStatusAt(guild, 120, 1000, unknown, () => false);
  assert.equal(status.kind, 'last-recorded');
  assert.equal(statusText(status, guild, names), `Last recorded ${dayLabel(99)} in Uneasy Truce`);
  // Past the prepared history: held through, later still being prepared.
  const pending = pendingSpans([], 300, { firstHour: 0, lastHour: 1000 });
  assert.deepEqual(pending, [{ firstHour: 301, lastHour: 1000 }]);
  const tail = balanceStatusAt(guild, 500, 1000, unknownHours([], [], 0, null, pending), hour => hour > 300);
  assert.equal(statusText(tail, guild, names), `Held through ${dayLabel(300)} in Uneasy Truce; later still being prepared`);
  assert.deepEqual(pendingSpans([], null, { firstHour: 0, lastHour: 10 }), [{ firstHour: 0, lastHour: 10 }], 'nothing prepared yet is pending, never empty');
});

test('names come from the reply, so a mod balance works; unresolved names say so and are counted', () => {
  const mod = balance('guild_struggle', [run(0, 10, '-1')]);
  assert.equal(statusText(balanceStatusAt(mod, 5, 10, [], () => false), mod, names), 'Uneasy Truce · 0.00 · toward the Merchants', 'the header rounds to two decimals; hover and table stay exact');
  const unnamed = balance('unknown_feud', [run(0, 10, '-50000')]);
  assert.equal(statusText(balanceStatusAt(unnamed, 5, 10, [], () => false), unnamed, {}), 'Range name unavailable · −0.50 · toward Side name unavailable');
  assert.deepEqual(nameWarnings([mod, unnamed], names), ['1 balance name unavailable', '1 range name unavailable']);
  // Authored bounds give the gridlines; a mod's narrower or wider ranges are drawn as authored.
  assert.deepEqual(balanceAxis(mod), { min: -1, max: 1, ticks: [-1, -0.25, 0.25, 1] });
  assert.deepEqual(balanceAxis({ ranges: [{ token: 'x', name: null, min: '-150000', max: '30000' }] }), { min: -1.5, max: 1, ticks: [-1.5, 0.3] });
  assert.deepEqual(balanceAxis(null).ticks, undefined);
});

const record = (fields: Partial<BalanceRecord>): BalanceRecord => ({ ordinal: 0, token: 'guild_struggle', name: 'Guild Struggle', value: '12345',
  left: { token: 'l', name: 'Left' }, right: { token: 'r', name: 'Right' }, trending: null, range: null, ranges: [], members: [], sources: [], ...fields });

test('the weekly change keeps exact signed sources and members, each totalled on its own and never reconciled', () => {
  const shared = record({ members: [{ tag: 'AAA', daily: '7', weekly: '1000' }, { tag: 'BBB', daily: '-3', weekly: '-250' }],
    sources: [{ token: 'mod_static', name: 'Guild charter', daily: '1', weekly: '600' }, { token: 'mod_static', name: 'Guild charter', daily: '1', weekly: '600' },
      { token: 'other_static', name: null, daily: '0', weekly: '-1000' }] });
  const parts = driftParts(shared, tag => ({ name: `Country ${tag}` }));
  assert.deepEqual(parts.sources!.map(part => [part.label, part.raw, part.text]), [
    ['Guild charter', 600n, '+0.006'], ['Guild charter', 600n, '+0.006'], ['Modifier name unavailable', -1000n, '−0.01']]);
  assert.equal(parts.weekly, 750n);
  assert.equal(parts.sourced, 200n, 'sources total on their own; no remainder is derived from the members');
  assert.equal('remainder' in parts, false);
  assert.deepEqual(parts.members.map(part => [part.label, part.raw, part.text, part.shareText]), [
    ['Country AAA', 1000n, '+0.01', '+0.00007 daily'], ['Country BBB', -250n, '−0.0025', '−0.00003 daily']]);
  const underived = driftParts(record({ members: [{ tag: 'AAA', daily: null, weekly: '5' }], sources: null }), tag => ({ name: tag }));
  assert.equal(underived.sources, null);
  assert.equal(underived.sourced, null, 'no source total without derived sources');
  assert.deepEqual(detailWarnings({ warnings: [], unavailable: [{ label: 'balance drift sources', reason: 'refused', count: 2 }], balances: [] }, record({ sources: null })),
    ['2 balance drift sources unavailable: refused', '1 balance drift source list unavailable']);
  const unread = driftParts(record({ members: [{ tag: 'AAA', daily: '1', weekly: null }, { tag: null, daily: '1', weekly: '2' }] }), tag => ({ name: tag }));
  assert.equal(unread.weekly, null, 'a member whose weekly value was not recorded leaves the total unknown, not zero');
  assert.equal(unread.members[0].value, null);
  assert.equal(unread.members[1].label, 'Country unavailable');
  assert.equal(exactValue('-100000'), '−1');
});

test('the record behind a country prefers the game\'s own, then the first in stored order listing it', () => {
  const first = record({ ordinal: 0, token: 'guild_struggle', members: [{ tag: 'AAA', daily: null, weekly: null }] });
  const second = record({ ordinal: 1, token: 'court_feud', members: [{ tag: 'AAA', daily: null, weekly: null }] });
  const third = record({ ordinal: 2, token: 'guild_struggle', members: [{ tag: 'AAA', daily: null, weekly: null }] });
  const reply = { balances: [first, second, third], current: [{ tag: 'AAA', record: 0 }] };
  assert.equal(recordFor(reply, null, 'AAA'), first);
  assert.equal(recordFor(reply, 'court_feud', 'AAA'), second);
  assert.equal(recordFor({ ...reply, current: null }, 'guild_struggle', 'AAA'), first);
  assert.equal(recordFor({ ...reply, current: null }, null, 'AAA'), null, 'unknown membership cannot say which the game shows');
  assert.equal(recordFor(reply, 'guild_struggle', 'ZZZ'), null);
});

test('Pin as widget keeps the balance and the picked day, or follows playback', () => {
  const source = { ...createWidget('balance-of-power', 'main'), countries: { kind: 'tags' as const, tags: ['MOD'] } };
  const track = { id: 'main', axisOffset: 0, branch: { firstHour: 100, lastHour: 500 } } as BoundTrack;
  const pinned = pinBalanceDrill(source, track, 250.6, false, 'guild_struggle');
  assert.equal(pinned.options?.show, 'weekly');
  assert.equal(pinned.options?.balance, 'guild_struggle');
  assert.equal(pinned.title, 'Balance of power: Weekly change');
  assert.deepEqual(resolveWindow(pinned, track, [track], 400), { firstHour: 250, lastHour: 250 });
  assert.deepEqual(pinBalanceDrill(source, track, 250, true, 'x').time, { mode: 'follow' });
  assert.equal(singleCountry(source.countries), true);
  assert.equal(singleCountry({ kind: 'all' }), false);
  assert.equal(singleCountry({ kind: 'tags', tags: ['A', 'B'] }), false);
  assert.deepEqual(createWidget('balance-of-power', 'main').countries, { kind: 'selected' });
});

test('5,000 countries are all listed, with no cap', () => {
  const holders = Array.from({ length: 5000 }, (_, index) => ({ holder: String(index), tags: [{ hour: 0, tag: `C${index}` }],
    balances: [balance('guild_struggle', [run(0, 100, String(index - 2500))], { record: [run(0, 100, String(index))] })], shown: [run(0, 100, 'guild_struggle')] }));
  const reply = validateHolderBalances(history({ holderBalances: holders, names }, 'balances'));
  const view = balanceRowsAt(reply, 50, () => true)!;
  assert.equal(view.rows.length, 5000);
  // One stored record listing all of them.
  const shared = balanceRowsAt({ holderBalances: holders.map(holder => ({ ...holder, balances: [balance('guild_struggle', [run(0, 100, '1')])] })), names, known: READ },
    50, () => true)!;
  assert.equal(shared.rows.length, 1);
  assert.equal(shared.rows[0].members.length, 5000);
});

test('a country view follows the holder of its tag at the date, through renames, never a reuser of the tag', () => {
  const renamed = holder('1', [[0, 'OLD'], [200, 'NEW']], [balance('guild_struggle', [run(0, 300, '1000')])]);
  const reuser = holder('2', [[200, 'OLD']], [balance('court_feud', [run(200, 300, '5')])]);
  const reply = { holderBalances: [renamed, reuser] };
  assert.equal(countryHolder(reply, 'OLD', 100)?.holder, '1');
  assert.equal(countryHolder(reply, 'OLD', 250)?.holder, '2', 'the tag now belongs to another holder');
  assert.equal(countryHolder(reply, 'NEW', 250)?.holder, '1', 'its whole history comes with the renamed holder');
  assert.equal(countryHolder(reply, 'NEW', 250)?.balances[0].values[0].firstHour, 0);
  assert.equal(countryHolder(reply, 'NEW', 100)?.holder, '1', 'before the tag was taken, the holder that first took it');
  assert.equal(countryHolder(reply, 'ZZZ', 100), null);
});

test('hours the balances were not read are unknown: a run ending there did not end then', () => {
  const guild = balance('guild_struggle', [run(0, 100, '1000')]);
  // Read through 100, not at 101..150, read again from 151 without the balance.
  const unread = unreadSpans([{ firstHour: 0, lastHour: 100 }, { firstHour: 151, lastHour: 400 }], 0, 400);
  assert.deepEqual(unread, [{ firstHour: 101, lastHour: 150 }]);
  const status = balanceStatusAt(guild, 120, 400, unknownHours([], [{ firstHour: 0, lastHour: 100 }, { firstHour: 151, lastHour: 400 }], 0, 400, []), () => false);
  assert.equal(status.kind, 'last-recorded');
  assert.deepEqual(unreadSpans([], 0, 10), [{ firstHour: 0, lastHour: 10 }], 'nothing read is all unknown');
  assert.deepEqual(unreadSpans([{ firstHour: 0, lastHour: 10 }], 0, null), [], 'nothing prepared yet is pending, not unread');
});

test('a recorded gap inside hours not read never ends a run early', () => {
  const guild = balance('guild_struggle', [run(0, 100, '1000')]);
  // Not read from 101 to 200, a recorded gap 101..150 among those hours.
  const unknown = unknownHours([{ firstHour: 101, lastHour: 150 }], [{ firstHour: 0, lastHour: 100 }, { firstHour: 201, lastHour: 400 }], 0, 400, []);
  assert.deepEqual(unknown, [{ firstHour: 101, lastHour: 200 }]);
  assert.equal(balanceStatusAt(guild, 175, 400, unknown, () => false).kind, 'last-recorded', 'not ended at 151');
  assert.deepEqual(balanceStatusAt(guild, 250, 400, unknown, () => false), { kind: 'ended', hour: 201, by: true, range: 'guild_struggle_mid' });
  // Touching stretches join too.
  assert.deepEqual(unknownHours([{ firstHour: 10, lastHour: 19 }], [], 0, null, [{ firstHour: 20, lastHour: 30 }]), [{ firstHour: 10, lastHour: 30 }]);
});

test('an hour the balances were not read lists nothing and claims no absence', () => {
  const guild = balance('guild_struggle', [run(0, 100, '1000')]);
  const known = [{ firstHour: 0, lastHour: 100 }, { firstHour: 151, lastHour: 400 }];
  const reply = { holderBalances: [holder('1', [[0, 'AAA']], [guild])], names, known };
  assert.equal(balanceRowsAt(reply, 120, () => true), null, 'not read: neither rows nor none');
  assert.equal(balanceRowsAt(reply, 50, () => true)?.rows.length, 1);
  assert.deepEqual(balanceRowsAt(reply, 200, () => true)?.rows, [], 'read with none active');
  const unknown = unknownHours([], known, 0, 400, []);
  assert.deepEqual(balanceAbsence(known, 120, 120, unknown, []), { kind: 'unread' });
  assert.deepEqual(balanceAbsence(known, 200, 200, unknown, []), { kind: 'none', recorded: true }, 'some hours before were not read');
  assert.deepEqual(balanceAbsence(known, 50, 50, unknown, []), { kind: 'none', recorded: false });
  assert.deepEqual(balanceAbsence(known, 500, 500, unknown, [{ firstHour: 401, lastHour: 600 }]), { kind: 'preparing' });
});

test('a followed holder\'s record is found by the tag it held at the detail\'s hour', () => {
  // Selected as NEW, the holder was OLD before 200; another holder takes OLD from 200.
  const renamed = holder('1', [[0, 'OLD'], [200, 'NEW']], [balance('guild_struggle', [run(0, 300, '1000')])]);
  const reuser = holder('2', [[200, 'OLD']], [balance('guild_struggle', [run(200, 300, '5')])]);
  const followed = countryHolder({ holderBalances: [renamed, reuser] }, 'NEW', 100)!;
  assert.equal(followed.holder, '1');
  const mine = record({ ordinal: 0, members: [{ tag: 'OLD', daily: null, weekly: null }] });
  const before = { hour: 100, balances: [mine], current: [{ tag: 'OLD', record: 0 }] };
  assert.equal(holderRecord(before, followed, null), mine, 'before the rename it is listed as OLD');
  assert.equal(holderRecord(before, followed, 'guild_struggle'), mine);
  const theirs = record({ ordinal: 1, members: [{ tag: 'OLD', daily: null, weekly: null }] });
  const own = record({ ordinal: 0, members: [{ tag: 'NEW', daily: null, weekly: null }] });
  const after = { hour: 250, balances: [own, theirs], current: [{ tag: 'NEW', record: 0 }, { tag: 'OLD', record: 1 }] };
  assert.equal(holderRecord(after, followed, null), own, 'after it, as NEW, never the reuser of OLD');
  assert.equal(holderRecord({ ...after, hour: -5 }, followed, null), null, 'no tag before its first read');
});

test('a weekly change waits for the history to cover its holder, and an unread detail hour is not recorded', () => {
  // Selected as NEW, which the holder took from OLD at 200; the details at 100 are already loaded.
  const renamed = holder('1', [[0, 'OLD'], [200, 'NEW']], [balance('guild_struggle', [run(0, 300, '1000')])]);
  const mine = record({ ordinal: 0, members: [{ tag: 'OLD', daily: null, weekly: null }] });
  const detail = { hour: 100, balances: [mine], current: [{ tag: 'OLD', record: 0 }], unavailable: [] };
  const full = history({ holderBalances: [renamed] });
  assert.deepEqual(weeklyState(full, 'NEW', 250, detail, null), { kind: 'record', record: mine });
  // A cold history knows no holder yet: never "none active".
  assert.deepEqual(weeklyState(history({ holderBalances: [] }, 'country', null), 'NEW', 250, detail, null), { kind: 'preparing' });
  // A prefix through 150 has not reached the rename that makes NEW this holder.
  const partial = history({ holderBalances: [holder('1', [[0, 'OLD']], renamed.balances)], pending: [{ firstHour: 151, lastHour: 400 }],
    coverage: { firstHour: 0, lastHour: 150, complete: false } }, 'country', 150);
  assert.deepEqual(weeklyState(partial, 'NEW', 250, detail, null), { kind: 'preparing' });
  // Pinned at 100 with the prefix through 150: both hours are covered, but no holder has
  // taken NEW yet in the prefix, so a later rename could still name one.
  const unrenamed = history({ holderBalances: [], pending: [{ firstHour: 151, lastHour: 400 }],
    coverage: { firstHour: 0, lastHour: 150, complete: false } }, 'country', 150);
  assert.deepEqual(weeklyState(unrenamed, 'NEW', 100, detail, null), { kind: 'preparing' });
  // The balance tables were not read at the detail's hour: not recorded, whatever the history says.
  const unread = { hour: 100, balances: [], current: null, unavailable: [{ label: 'balances of power', reason: 'not recorded', count: 1 }] };
  assert.deepEqual(weeklyState(full, 'NEW', 250, unread, null), { kind: 'unread' });
  assert.deepEqual(weeklyState(history({ holderBalances: [] }, 'country', null), 'NEW', 250, unread, null), { kind: 'unread' });
  // Read, covered and holding none: none active.
  assert.deepEqual(weeklyState(full, 'NEW', 250, { ...detail, balances: [], current: [] }, null), { kind: 'none' });
});
