import assert from 'node:assert/strict';
import test from 'node:test';
import { formatDateTime } from '../../dashboard/calendar.ts';
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
  const withdrawn = project([{ line: 'contract', ...contract, request: true, status: 'requestEnded' }, { line: 'event', key: '4', contract: '18', seller: 'GER', buyer: 'FRA', hour: 109, kind: 'requestEnded', variant: null, equipment: null, equipmentMissing: null, amountRaw: null, amountNotRecorded: false, reason: null }], ['group:18']).rows[1]!;
  const statusText = withdrawn.cells.status.value.kind === 'known' ? withdrawn.cells.status.value.value : null;
  const itemText = withdrawn.cells.item.value.kind === 'known' ? withdrawn.cells.item.value.value : null;
  assert.deepEqual([withdrawn.name, itemText, statusText, withdrawn.muted], ['Request ended', 'Request ended', 'Request ended', true], 'a request removal is named as one, never as a contract ending');
});

test('the line after a capped contract offers the rest, and toggles back once every event is listed', () => {
  const more = (hidden: string) => project([{ line: 'more', key: 'more:18', hour: 100, contract: '18', total: '41', hidden }], ['group:18']).rows[0]!;
  assert.deepEqual([more('9').key, more('9').pageParent, more('9').name], ['all:group:18', 'group:18', 'Show all 41 events · 9 more']);
  assert.equal(more('0').name, 'Show fewer events');
});

test('payments and deliveries seen only after an unrecorded start or across a gap are lower bounds with their reason', () => {
  const reason = 'The contract began before its first recorded event';
  const row = project([{ line: 'contract', ...contract, hour: 103, observed: 105, exact: false, startedHour: null, paidMissing: reason, deliveryMissing: reason, items: [{ ...contract.items[0]!, deliveryUnavailable: reason }] }], []).rows[0]!;
  const extras = typeof row.tooltipExtra === 'function' ? row.tooltipExtra() : row.tooltipExtra;
  assert.deepEqual(extras?.find(extra => extra.label === 'First seen')?.value, { kind: 'known', value: formatDateTime(105) }, 'a contract whose start was not recorded was first seen when its first event was read');
  assert.deepEqual(row.cells.paid.value, { kind: 'partial', value: '−48', reason, atLeast: true });
  assert.deepEqual(row.cells.delivered.value, { kind: 'partial', value: '250', reason, atLeast: true });
  const unseen = project([{ line: 'contract', ...contract, paidRaw: null, paidMissing: reason, deliveryMissing: reason, items: [{ ...contract.items[0]!, deliveredRaw: null, deliveryUnavailable: reason }] }], []).rows[0]!;
  assert.deepEqual([unseen.cells.paid.value.kind, unseen.cells.delivered.value.kind], ['notRecorded', 'notRecorded'], 'nothing witnessed is missing, never zero');
});

test('missing equipment and quote names preserve variant tokens and missing reasons without faking known values (#823 S14)', () => {
  const rows: MarketLedgerLine[] = [
    {
      line: 'contract',
      key: '42',
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
      items: [
        {
          variant: '99',
          name: null,
          nameMissing: 'Equipment design table was not read',
          icon: null,
          deliveredRaw: null,
          contractedRaw: '50000000',
          deliveryUnavailable: 'Deliveries were not recorded during an unread gap',
        },
        {
          variant: '100',
          name: 'Field Kit',
          nameMissing: null,
          icon: null,
          deliveredRaw: null,
          contractedRaw: '10000000',
          deliveryUnavailable: null,
        },
      ],
      deliveryMissing: null,
      paidRaw: null,
      paidMissing: null,
      quotes: [{
        variant: '99',
        name: null,
        nameMissing: 'Quoted design name not recorded',
        priceRaw: '1200000',
      }],
      termsMissing: null,
      eventsTotal: '3',
    },
    {
      line: 'event',
      key: 'd1',
      contract: '42',
      seller: 'GER',
      buyer: 'FRA',
      hour: 104,
      kind: 'delivery',
      variant: '99',
      equipment: null,
      equipmentMissing: 'Delivered equipment name not recorded',
      amountRaw: '10000000',
      amountNotRecorded: false,
      reason: null,
    },
    {
      line: 'event',
      key: 'd2',
      contract: '42',
      seller: 'GER',
      buyer: 'FRA',
      hour: 106,
      kind: 'deliveryUnavailable',
      variant: '99',
      equipment: null,
      equipmentMissing: 'Unavailable delivery equipment name not recorded',
      amountRaw: null,
      amountNotRecorded: true,
      reason: 'Transfer count was not recorded',
    },
    {
      line: 'event',
      key: 'd3',
      contract: '42',
      seller: 'GER',
      buyer: 'FRA',
      hour: 107,
      kind: 'delivery',
      variant: '100',
      equipment: 'Field Kit',
      equipmentMissing: null,
      amountRaw: null,
      amountNotRecorded: true,
      reason: 'The contract of this event was not recorded.',
    },
  ];
  const model = project(rows, ['group:42']);
  const [contractRow, deliveryRow, unavailableRow, missingAmountRow] = model.rows;
  assert.deepEqual(contractRow!.cells.item?.value, { kind: 'notRecorded', reason: 'Equipment design table was not read' });
  assert.equal(contractRow!.cells.item?.inlineSubtext, 'Design 99 · +1 more', 'contract item preserves variant token alongside missing reason');

  assert.deepEqual(deliveryRow!.cells.item?.value, { kind: 'notRecorded', reason: 'Delivered equipment name not recorded' }, 'delivery event never fakes knownValue("Delivery") when equipment is missing');
  assert.equal(deliveryRow!.cells.item?.inlineSubtext, 'Design 99 · Delivery');
  assert.deepEqual(unavailableRow!.cells.item?.value, { kind: 'notRecorded', reason: 'Unavailable delivery equipment name not recorded' });
  assert.equal(unavailableRow!.cells.item?.inlineSubtext, 'Design 99 · Delivery');
  assert.deepEqual(unavailableRow!.cells.delivered?.value, { kind: 'notRecorded', reason: 'Transfer count was not recorded' });
  assert.deepEqual(
    missingAmountRow!.cells.delivered?.value,
    { kind: 'notRecorded', reason: 'The contract of this event was not recorded.' },
    'delivery child with missing amount preserves event.reason',
  );

  const extra = (typeof contractRow!.tooltipExtra === 'function' ? contractRow!.tooltipExtra() : contractRow!.tooltipExtra) ?? [];
  assert.ok(extra.some(r => r.label === 'Price (Design 99)' && r.value.kind === 'known'), 'quote price keeps variant token');
  assert.ok(extra.some(r => r.label === 'Design 99' && r.value.kind === 'notRecorded' && r.value.reason === 'Quoted design name not recorded'), 'quote missing-name reason is preserved as notRecorded');
  assert.ok(extra.some(r => r.label === 'Design 99 name' && r.value.kind === 'notRecorded' && r.value.reason === 'Equipment design table was not read'), 'item missing-name reason is preserved as notRecorded');
  const itemRow = extra.find(r => r.label === 'Design 99' && r.value.kind === 'notRecorded' && r.value.reason === 'Deliveries were not recorded during an unread gap');
  assert.ok(itemRow, 'item deliveryUnavailable uses notRecorded rather than knownValue("Not recorded (...)")');
  const zeroDeliveredRow = extra.find(r => r.label === 'Field Kit' && r.value.kind === 'known' && r.value.value === '0 / 100 units');
  assert.ok(zeroDeliveredRow, 'null deliveredRaw without deliveryUnavailable preserves #1152 zero-delivered progress');
});
