import test from 'node:test';
import assert from 'node:assert/strict';
import type { LogCell } from '../generated/dashboard';
import { ledgerEntry, ledgerFacets, ledgerFigure, ledgerSelection, ledgerTitle, shownKinds } from './marketLedger.ts';

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
});

test('missing or unrecorded market cells stay null instead of defaulting to zero or string zero', () => {
  const unrecorded = row({
    kind: { kind: 'notRecorded' },
    contract: { kind: 'notRecorded' },
    variant: { kind: 'notRecorded' },
  });
  assert.equal(unrecorded.item.kind, null);
  assert.equal(unrecorded.item.contract, null);
  assert.equal(unrecorded.item.variant, null);
});