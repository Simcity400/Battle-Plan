import assert from 'node:assert/strict';
import test from 'node:test';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { countryBalances, decodeTransfers } from './territoryData.ts';
import type { TransferOccurrence } from './territoryData';
import { countryTooltip, transferRequest, transferTooltip } from './territoryTooltip.ts';
import type { TransferLabels } from './territoryTooltip';
import { deal, holder, reply } from './transfers.fixture.ts';
import type { Move } from './transfers.fixture.ts';

const names: Record<string, string> = { LOS: 'Loserland', WIN: 'Winnerland', NEW: 'Freedonia', OTH: 'Otherland' };
const labels: TransferLabels = {
  country: side => ({ name: names[side.tag] ?? 'Country name unavailable', flag: `flag:${side.tag}` }),
  tag: tag => ({ name: names[tag] ?? 'Country name unavailable' }),
  state: occurrence => `State name ${occurrence.state}`,
};
const move = (state: number, from: Move['from'], to: Move['to'], after: number, extra: Partial<Move> = {}): Move => ({ state, from, to, before: after - 1, after, ...extra });
const los = holder(1, 'LOS', 100), win = holder(2, 'WIN', 101), free = holder(3, 'NEW', 101), oth = holder(4, 'OTH', 100);
const data = decodeTransfers(reply([deal('peace', [
  move(1, los, win, 101, { vp: '5' }), move(2, los, win, 101, { vp: '4' }), move(6, oth, win, 101, { vp: '1' }),
  move(3, los, free, 101, { vp: null, released: true }),
], { warName: 'Mod Frontier War' })]));
const occurrence = (state: number) => data.all.find(item => item.state === state)!;

test('a country tooltip breaks gains and losses down by counterpart, without a wall of per-state rows', () => {
  const balances = new Map(countryBalances(data.all).map(row => [row.side.tag, row]));
  const winner = countryTooltip(balances.get('WIN')!, labels);
  assert.equal(winner.title, 'Winnerland');
  assert.deepEqual(winner.rows.map(row => [row.label, row.value ?? null]), [['Gained', '3 states · 10 VP'], ['Loserland', '2'], ['Otherland', '1']]);
  assert.equal(winner.countryFlag?.flag, 'flag:WIN');
  const loser = countryTooltip(balances.get('LOS')!, labels);
  // One unknown state's VP leaves that side's total out rather than a partial sum.
  assert.deepEqual(loser.rows.map(row => [row.label, row.value ?? null]), [['Lost', '3 states'], ['Winnerland', '2'], ['Freedonia', '1']]);
});

test('an exact transfer gives its hour; a comparison of two reads says "by" and when it was still unchanged', () => {
  const exactData = decodeTransfers(reply([deal('exact', [move(7, los, win, 101, { before: 101, derived: false })], { warName: null })]));
  const exact = transferTooltip(exactData.all[0], exactData.deals.get('exact'), labels);
  assert.equal(exact.title, 'State name 7');
  assert.equal(exact.subtitle, 'Loserland → Winnerland');
  assert.deepEqual(exact.rows.map(row => [row.label, row.value, row.tone ?? null]), [
    ['Date', formatDateTime(101), null], ['War', 'Not recorded', 'muted'], ['Victory points', '1', null]]);
  assert.equal(exact.description, undefined);
  const derived = transferTooltip(occurrence(1), data.deals.get('peace'), labels);
  assert.equal(derived.rows[0].value, `By ${formatDateTime(101)}`);
  assert.equal(derived.rows[1].value, 'Mod Frontier War');
  assert.equal(derived.description, `It changed hands after ${formatDateTime(100)}; the exact hour was not recorded.`);
});

test('how appears only when the settlement records it; unknown and pending VP stay one muted value', () => {
  const peace = data.deals.get('peace')!;
  const withSubject = { ...peace, items: [...peace.items, { derived: true, dates: { beforeHour: 100, afterHour: 101 }, winner: 'WIN', action: { kind: 'newSubject' as const, key: null, name: null },
    change: { kind: 'newSubject' as const, overlord: 'WIN', subject: 'NEW', autonomyState: 'mod_level', startHour: 101 } }] };
  const released = transferTooltip(occurrence(3), withSubject, labels);
  assert.deepEqual(released.rows.find(row => row.label === 'How')?.value, 'Released, subject of Winnerland');
  assert.deepEqual(released.rows.at(-1), { label: 'Victory points', value: 'Not recorded', tone: 'muted' });
  assert.equal(transferTooltip(occurrence(1), peace, labels).rows.some(row => row.label === 'How'), false);
  const pending: TransferOccurrence = { ...occurrence(1), victoryPoints: null, victoryPointsPending: true };
  assert.equal(transferTooltip(pending, peace, labels).rows.at(-1)?.value, 'Not read yet');
});

test('requests name each country and state at its own read', () => {
  const request = transferRequest([occurrence(3)], countryBalances(data.all).filter(row => row.side.tag === 'WIN'));
  assert.deepEqual(request.states, [{ hour: 101, state: 3 }]);
  assert.deepEqual(request.countries.map(country => `${country.tag}@${country.hour}`).sort(), ['LOS@100', 'NEW@101', 'OTH@100', 'WIN@101']);
});
