import assert from 'node:assert/strict';
import test from 'node:test';
import { countryBalances, decodeTransfers } from './territoryData.ts';
import type { TransferOccurrence } from './territoryData';
import { transferRequest, transferTooltip } from './territoryTooltip.ts';
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
