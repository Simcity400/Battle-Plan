import test from 'node:test';
import assert from 'node:assert/strict';
import type { MarketHistoryReply } from '../../generated/dashboard';
import { validateMarketHistory } from '../../dashboard/marketHistory.ts';
import { agreementRows, amountOf, openAgreement } from './agreementsData.ts';
import { CLOSED, reduceDrill } from '../../charts/drill/drillState.ts';

const item = (pool: number, variant: string, amount: string, from = 10, to = 30) => ({ pool, ordinal: 0, variant, amounts: [[from, to, amount]] });
const reply = (): MarketHistoryReply => ({
  branchId: '1', view: 'agreements', lastHour: 50, known: [[0, 50, '1']], contracts: [], offers: [], warnings: [],
  holders: { '1': [{ hour: 0, tag: 'GIV' }], '2': [{ hour: 0, tag: 'REC' }] },
  variants: { a1: [{ firstHour: 0, name: 'Synthetic rifle', known: true, design: null }] },
  agreements: [{
    giver: '1', id: '0000000000000004', receiver: [[10, 30, '2']], revision: [[10, 30, '0']], since: [[10, 30, '9000']], last: [[10, 30, '9100']],
    items: [item(0, 'a1', '6000000'), item(3, 'a1', '12000000'), item(4, 'a1', '0'), item(1, 'b2', '20000')],
    fuel: [[[10, 30, '32768']], [], [[10, 30, '65536']], [], []],
  }],
  coverage: { firstHour: 0, lastHour: 50, complete: true }, pending: [],
} as unknown as MarketHistoryReply);

test('agreements show schedules, retained totals and fuel in their own units, by direction', () => {
  const data = validateMarketHistory('agreements')(reply());
  const sent = agreementRows(data, 'sent', tag => tag === 'GIV', 20, true).rows;
  assert.equal(sent.length, 2);
  const [equipment, fuel] = sent;
  assert.equal(equipment.partner, 'REC');
  assert.deepEqual(equipment.items.map(entry => [entry.name, entry.schedule]), [['Synthetic rifle', 'Monthly 60'], [null, 'Production share 20%']]);
  assert.equal(amountOf(equipment, equipment.items[0].delivered), '120');
  // A recorded zero loss is 0, an item with no loss pool entry is unknown.
  assert.equal(amountOf(equipment, equipment.items[0].lost), '0');
  assert.equal(amountOf(equipment, equipment.items[1].lost), '—');
  assert.equal(fuel.items[0].schedule, 'Daily 1');
  assert.equal(amountOf(fuel, fuel.items[0].delivered), '2');
  assert.deepEqual(agreementRows(data, 'received', tag => tag === 'GIV', 20, true), { rows: [], unattributed: 0 });
  const received = agreementRows(data, 'received', tag => tag === 'REC', 20, true).rows[0];
  assert.deepEqual([received.from, received.partner, received.giver, received.receiver], ['REC', 'GIV', 'GIV', 'REC']);
  assert.deepEqual([equipment.giver, equipment.receiver], ['GIV', 'REC']);
});

test('fuel already sent keeps its row after the fuel terms stop', () => {
  const stopped = reply();
  stopped.agreements[0].fuel = [[[10, 19, '32768'], [20, 30, '0']], [[10, 30, '0']], [[10, 30, '65536']], [[10, 30, '0']], []];
  const { rows } = agreementRows(validateMarketHistory('agreements')(stopped), 'sent', tag => tag === 'GIV', 25, true);
  assert.equal(rows.length, 2);
  const fuel = rows[1];
  assert.equal(fuel.fuel, true);
  assert.equal(fuel.items[0].schedule, '');
  assert.equal(amountOf(fuel, fuel.items[0].delivered), '2');
});

test('ended agreements keep their last read and can be hidden; later ones are not shown early', () => {
  const data = validateMarketHistory('agreements')(reply());
  const ended = agreementRows(data, 'sent', () => true, 40, true).rows;
  assert.ok(ended.length && ended.every(row => row.ended && row.at === 30));
  assert.equal(agreementRows(data, 'sent', () => true, 40, false).rows.length, 0);
  assert.equal(agreementRows(data, 'sent', () => true, 5, true).rows.length, 0);
});

test('an agreement whose receiver has no identity stands, sent by its giver and unattributed when received', () => {
  const unnamed = reply();
  // The receiver is identified until 19; the agreement stands, with its revision read, through 30.
  unnamed.agreements[0].receiver = [[10, 19, '2']];
  const data = validateMarketHistory('agreements')(unnamed);
  const sent = agreementRows(data, 'sent', tag => tag === 'GIV', 25, true);
  assert.deepEqual([sent.rows.length, sent.rows[0].partner, sent.rows[0].receiver, sent.rows[0].ended, sent.unattributed], [2, null, null, false, 0]);
  // Any country may be its receiver: no row lists it, and none proves there is none.
  assert.deepEqual(agreementRows(data, 'received', tag => tag === 'REC', 25, true), { rows: [], unattributed: 1 });
  assert.equal(agreementRows(data, 'received', tag => tag === 'REC', 15, true).rows.length, 2);
  // Ended after 30, its last read still names no receiver.
  assert.equal(agreementRows(data, 'received', () => true, 40, true).unattributed, 1);
});

test('an ended agreement opens at its last read, never following the playhead past it', () => {
  const [ended] = agreementRows(validateMarketHistory('agreements')(reply()), 'sent', () => true, 40, true).rows;
  assert.ok(ended.ended);
  for (const playing of [true, false]) assert.deepEqual(reduceDrill(CLOSED, openAgreement(ended, 'agreement', playing, 40)),
    { breakdown: 'agreement', follow: false, hour: 30 });
  const [active] = agreementRows(validateMarketHistory('agreements')(reply()), 'sent', () => true, 20, true).rows;
  assert.equal(reduceDrill(CLOSED, openAgreement(active, 'agreement', true, 20)).follow, true, 'an active one follows playback as before');
});
