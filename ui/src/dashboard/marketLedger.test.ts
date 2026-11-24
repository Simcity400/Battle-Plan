import test from 'node:test';
import assert from 'node:assert/strict';
import type { LogCell } from '../generated/dashboard';
import { ledgerEntry, ledgerFacets, ledgerFigure, ledgerPartyName, ledgerSelection, ledgerTitle, shownKinds } from './marketLedger.ts';

const row = (cells: Record<string, LogCell>) => ledgerEntry({ key: '9', hour: 40, cells, fields: {} });
const number = (value: string): LogCell => ({ kind: 'number', value });
const textCell = (value: string): LogCell => ({ kind: 'text', value });

test('a country\'s ledger asks either party\'s facet by its tags; every country asks the kinds alone', () => {
  assert.deepEqual(ledgerFacets([11, 20], ['GER']), [{ name: 'kind', values: [11, 20], countries: [] }, { name: 'country|receiver', values: [], countries: ['GER'] }]);
  assert.deepEqual(ledgerFacets([11], null), [{ name: 'kind', values: [11], countries: [] }]);
  assert.deepEqual(shownKinds(key => key !== 'payments'), [10, 11, 12, 20, 34], 'unavailable amounts follow deliveries');
  assert.deepEqual(shownKinds(key => key === 'payments'), [21]);
});

test('an unavailable amount is named, never zero; amounts read in their units; a row opens its contract or agreement', () => {
  const unavailable = row({ kind: number('34'), mode: number('20'), contract: textCell('0000000000000012'), seller: textCell('FRA'), buyer: { kind: 'missing' },
    amountRaw: { kind: 'missing' }, reason: textCell('Delivery amount unavailable: cumulative baseline uncertified'), variant: textCell('0') });
  assert.deepEqual(ledgerFigure(unavailable.item), { text: 'Amount unavailable', negative: false });
  assert.equal(ledgerTitle(unavailable.item, tag => tag ?? 'Unknown'), 'Delivery amount unavailable');
  assert.equal(ledgerSelection(unavailable.item), 'contract:false:0000000000000012');
  const fuel = row({ kind: number('5'), contract: textCell('00000000000000a1'), seller: textCell('GER'), buyer: textCell('FRA'), amountRaw: number('65536'), auxiliaryRaw: number('32768') });
  assert.deepEqual(ledgerFigure(fuel.item), { text: '2 fuel · 1 lost', negative: false });
  assert.equal(ledgerTitle(fuel.item, tag => tag ?? 'Unknown'), 'Fuel lent to FRA');
  assert.equal(ledgerSelection(fuel.item), 'agreement:00000000000000a1');
  const payment = row({ kind: number('21'), amountRaw: number('-150000'), buyer: textCell('GER') });
  assert.deepEqual(ledgerFigure(payment.item), { text: '-1.5 CIC', negative: true });
  assert.equal(ledgerFigure(row({ kind: number('12') }).item), null);
  const unrecorded = row({ kind: { kind: 'notRecorded' }, contract: { kind: 'notRecorded' }, variant: { kind: 'notRecorded' },
    seller: { kind: 'notRecorded' }, buyer: { kind: 'notRecorded' }, equipmentMissing: textCell('Equipment design not recorded') });
  assert.equal(unrecorded.item.kind, null);
  assert.equal(unrecorded.item.contract, null);
  assert.equal(unrecorded.item.variant, null);
  assert.equal(ledgerTitle(unrecorded.item, tag => tag ?? 'Unknown'), 'Market event kind not recorded');
  assert.equal(ledgerSelection(unrecorded.item), null);
});

test('missing vs not-recorded parties, unread delivery amounts, and mode-unknown kind 34 stay distinct', () => {
  const delivery = row({ kind: number('20'), seller: { kind: 'missing' }, buyer: { kind: 'notRecorded' }, amountRaw: { kind: 'notRecorded' } });
  assert.equal(delivery.item.sellerState, 'unavailable');
  assert.equal(delivery.item.buyerState, 'notRecorded');
  assert.equal(ledgerPartyName(delivery.item.seller, delivery.item.sellerState, t => t), 'Country name unavailable');
  assert.equal(ledgerPartyName(delivery.item.buyer, delivery.item.buyerState, t => t), 'Country not recorded');
  assert.deepEqual(ledgerFigure(delivery.item), { text: 'Amount not recorded', negative: false });
  assert.equal(ledgerTitle(delivery.item, (tag, state) => ledgerPartyName(tag, state ?? 'notRecorded', t => t)), 'Delivery to Country not recorded');

  const partialFuel = row({ kind: number('5'), amountRaw: number('65536'), auxiliaryRaw: { kind: 'notRecorded' } });
  assert.deepEqual(ledgerFigure(partialFuel.item), { text: '2 fuel · lost not recorded', negative: false });

  const unknownMode34 = row({ kind: number('34'), mode: { kind: 'notRecorded' }, contract: textCell('00000000000000ff') });
  assert.equal(ledgerTitle(unknownMode34.item, tag => tag ?? 'Unknown'), 'Delivery amount unavailable');
  assert.equal(ledgerSelection(unknownMode34.item), null, 'mode-unknown kind 34 never misroutes to an agreement');
  assert.deepEqual(ledgerFigure(unknownMode34.item), { text: 'Amount unavailable', negative: false });
});