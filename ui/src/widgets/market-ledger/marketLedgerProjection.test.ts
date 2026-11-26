import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { buildRowSourceFromHead } from '../../families/table/rowSource.ts';
import type { MarketContractEvent, MarketLedgerTailReply } from '../../generated/dashboard';
import { marketLedgerProjection } from './marketLedgerProjection.ts';

const events: MarketContractEvent[] = [
  { key: '3', hour: 108, kind: 'payment', variant: null, equipment: null, equipmentMissing: null, amountRaw: '4800000', amountNotRecorded: false, reason: null },
  { key: '2', hour: 105, kind: 'delivery', variant: '6', equipment: 'Brasswick Rifle Set', equipmentMissing: null, amountRaw: '25000000', amountNotRecorded: false, reason: null },
];

const reply = (listed: boolean): MarketLedgerTailReply => ({
  branchId: '1',
  offset: '0',
  total: '1',
  live: '1',
  counts: [{ key: 'active', total: '1' }],
  nextChange: null,
  rows: [{
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
    events: listed ? events : [],
  }],
  missing: [],
  warnings: [],
});

const project = (listed: boolean, expanded: readonly string[]) => marketLedgerProjection.project(
  createBoundedReadView({ hour: 110, rows: new Map([['main', buildRowSourceFromHead('mkt', reply(listed), undefined, 110)!]]) }),
  { perspective: 'FRA', selectionEmpty: null, expanded },
);

test('a collapsed contract is a closed group that counts its events, and opening it lists them', () => {
  const closed = project(false, []).rows[0]!;
  assert.deepEqual(closed.pageGroup, { members: 2, open: false }, 'the row says how many events it holds without carrying them');
  assert.equal(closed.children, undefined);
  const opened = project(true, ['group:18']).rows[0]!;
  assert.deepEqual(opened.pageGroup, { members: 2, open: true });
  assert.deepEqual(opened.children?.map(child => child.key), ['ev:3', 'ev:2']);
  assert.deepEqual(closed.cells, opened.cells, 'opening a contract changes none of its row');
});
