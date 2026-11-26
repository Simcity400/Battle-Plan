import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { buildRowSourceFromHead } from '../../families/table/rowSource.ts';
import type { MarketContractRow, MarketLedgerLine, MarketLedgerTailReply } from '../../generated/dashboard';
import { marketLedgerProjection } from './marketLedgerProjection.ts';

const events: MarketLedgerLine[] = [
  { line: 'event', key: '3', contract: '18', seller: 'GER', buyer: 'FRA', hour: 108, kind: 'payment', variant: null, equipment: null, equipmentMissing: null, amountRaw: '4800000', amountNotRecorded: false, reason: null },
  { line: 'event', key: '2', contract: '18', seller: 'GER', buyer: 'FRA', hour: 105, kind: 'delivery', variant: '6', equipment: 'Brasswick Rifle Set', equipmentMissing: null, amountRaw: '25000000', amountNotRecorded: false, reason: null },
];

const contract: MarketContractRow = {
  key: '18',
  hour: 100,
  observed: 100,
  exact: true,
  ordinal: '0',
  live: true,
  request: false,
  requestedHour: null,
  startedHour: 100,
  endedHour: null,
  status: 'active',
  seller: 'GER',
  sellerNotRecorded: false,
  buyer: 'FRA',
  buyerNotRecorded: false,
  items: [{ variant: '6', name: 'Brasswick Rifle Set', nameMissing: null, icon: null, deliveredRaw: '25000000', contractedRaw: '50000000', deliveryUnavailable: null }],
  deliveryMissing: null,
  paidRaw: '4800000',
  paidMissing: null,
  quotes: [],
  termsMissing: null,
  eventsTotal: '2',
};

const reply = (rows: MarketLedgerLine[]): MarketLedgerTailReply => ({
  branchId: '1',
  offset: '0',
  total: String(rows.length),
  live: '1',
  counts: [{ key: 'active', total: '1' }],
  nextChange: null,
  rows,
  missing: [],
  warnings: [],
});

const project = (rows: MarketLedgerLine[], expanded: readonly string[]) => marketLedgerProjection.project(
  createBoundedReadView({ hour: 110, rows: new Map([['main', buildRowSourceFromHead('mkt', reply(rows), undefined, 110)!]]) }),
  { perspective: 'FRA', selectionEmpty: null, expanded },
);

test('a collapsed contract is a closed group that counts its events, and opening it pages them as lines after it', () => {
  const closed = project([{ line: 'contract', ...contract }], []).rows;
  assert.deepEqual(closed[0]!.pageGroup, { members: 2, open: false }, 'the row says how many events it holds without carrying them');
  assert.equal(closed.length, 1);
  const opened = project([{ line: 'contract', ...contract }, ...events], ['group:18']).rows;
  assert.deepEqual(opened[0]!.pageGroup, { members: 2, open: true });
  assert.deepEqual(opened.slice(1).map(row => [row.key, row.pageParent]), [['ev:3', 'group:18'], ['ev:2', 'group:18']]);
  assert.equal(opened[1]!.cells.status.value.kind === 'known' ? opened[1]!.cells.status.value.value : null, 'Paid', 'an event line is labelled from its own parties');
  assert.deepEqual(closed[0]!.cells, opened[0]!.cells, 'opening a contract changes none of its row');
});

test('the line after a capped contract offers the rest, and toggles back once every event is listed', () => {
  const more = (hidden: string) => project([{ line: 'more', key: 'more:18', hour: 100, contract: '18', total: '41', hidden }], ['group:18']).rows[0]!;
  assert.deepEqual([more('9').key, more('9').pageParent, more('9').name], ['all:group:18', 'group:18', 'Show all 41 events · 9 more']);
  assert.equal(more('0').name, 'Show fewer events');
});

test('payments and deliveries seen only after an unrecorded start or across a gap are lower bounds with their reason', () => {
  const reason = 'The contract began before its first recorded event';
  const row = project([{ line: 'contract', ...contract, exact: false, startedHour: null, paidMissing: reason, deliveryMissing: reason, items: [{ ...contract.items[0]!, deliveryUnavailable: reason }] }], []).rows[0]!;
  assert.deepEqual(row.cells.paid.value, { kind: 'partial', value: '−48', reason, atLeast: true });
  assert.deepEqual(row.cells.delivered.value, { kind: 'partial', value: '250', reason, atLeast: true });
  const unseen = project([{ line: 'contract', ...contract, paidRaw: null, paidMissing: reason, deliveryMissing: reason, items: [{ ...contract.items[0]!, deliveredRaw: null, deliveryUnavailable: reason }] }], []).rows[0]!;
  assert.deepEqual([unseen.cells.paid.value.kind, unseen.cells.delivered.value.kind], ['notRecorded', 'notRecorded'], 'nothing witnessed is missing, never zero');
});
