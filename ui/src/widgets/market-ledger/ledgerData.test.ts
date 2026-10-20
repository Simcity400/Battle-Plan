import test from 'node:test';
import assert from 'node:assert/strict';
import type { CountryGroupReply, MarketLedgerReply } from '../../generated/dashboard';
import type { CountrySelection } from '../../dashboard/model';
import { extendCursors, ledgerCountries, ledgerCursors, ledgerEmpty, ledgerFigure, ledgerRows, ledgerTitle, shownKinds, validateLedger } from './ledgerData.ts';

const row = (id: string, kind: number, amountRaw: string | null, reason: string | null = null) => ({ id, occurrence: id, hour: 20, kind, mode: kind === 34 ? 20 : 0,
  contract: '0000000000000009', seller: '1', buyer: '2', variant: 'a1', amountRaw, totalRaw: '0', reason });
const reply = (extra: Partial<MarketLedgerReply> = {}): MarketLedgerReply => validateLedger({ branchId: '1', lastHour: 40, total: '3', next: null, known: [[0, 40, '1']], warnings: [],
  holders: { '1': [{ hour: 0, tag: 'SEL' }], '2': [{ hour: 0, tag: 'BUY' }] },
  variants: { a1: [{ firstHour: 0, name: 'Synthetic rifle', known: true, design: null }] },
  rows: [row('3', 21, '150000'), row('2', 34, null, 'Delivery amount unavailable: cumulative baseline uncertified'), row('1', 20, '-200000')],
  coverage: { firstHour: 0, lastHour: 40, complete: true }, pending: [], ...extra });

test('ledger rows name both parties at their hour and keep unavailable amounts apart from zero', () => {
  const rows = ledgerRows(reply(), 0);
  const names = (tag: string | null) => tag ?? 'unknown';
  assert.deepEqual(rows.map(item => ledgerTitle(item.item, names)), ['Payment collected from BUY', 'Delivery amount unavailable', 'Delivery to BUY']);
  assert.deepEqual(rows.map(item => ledgerFigure(item.item)), [{ text: '1.5 CIC', negative: false }, { text: 'Amount unavailable', negative: false }, { text: '-2', negative: true }]);
  assert.equal(rows[2].item.equipment, 'Synthetic rifle');
});

test('kind toggles map to recorded kinds; unavailable deliveries follow deliveries', () => {
  assert.deepEqual(shownKinds(key => key === 'deliveries'), [20, 34]);
  assert.deepEqual(shownKinds(key => key === 'payments'), [21]);
});

test('a chosen-country ledger has no total and pages by each loaded page\'s cursor', () => {
  const first = reply({ total: null, next: 'a' });
  // Page 1 waits on page 0's cursor; page 2 is unknown until page 1 loads.
  assert.deepEqual(ledgerCursors(first, []), [undefined, 'a']);
  const loaded = new Map([[1, reply({ total: null, next: 'b' })], [2, reply({ total: null, next: null })]]);
  const next = extendCursors(first, [], index => loaded.get(index));
  assert.deepEqual(next, ['b']);
  // Page 2 is the oldest: nothing further opens.
  assert.deepEqual(ledgerCursors(first, next), [undefined, 'a', 'b']);
  assert.equal(extendCursors(first, next, index => loaded.get(index)), next);
  assert.deepEqual(ledgerCursors(reply({ total: null }), []), [undefined]);
  assert.throws(() => validateLedger({ ...reply(), known: [[0, 'x', '1']] }));
});

test('an empty ledger says no activity only where the market was read through the hour', () => {
  const empty = (known: [number, number, string][], lastHour = 40) => reply({ rows: [], total: null, known, lastHour });
  assert.equal(ledgerEmpty(empty([[0, 40, '1']]), 0, 30), 'none');
  assert.equal(ledgerEmpty(empty([[0, 10, '1'], [20, 40, '1']]), 0, 30), 'partial');
  assert.equal(ledgerEmpty(empty([[35, 40, '1']]), 0, 30), 'unrecorded');
  assert.equal(ledgerEmpty(empty([]), 0, 30), 'unrecorded');
  // Not prepared through the hour yet: neither empty nor unrecorded.
  assert.equal(ledgerEmpty(empty([[0, 20, '1']], 20), 0, 30), 'pending');
});

test('a group whose roster was never recorded is unknown, never "no members"', () => {
  const groups = (members: { tag: string; firstHour: number; lastHour: number }[], unobserved: { firstHour: number; lastHour: number }[]) =>
    ({ humans: [], wars: [], factions: [{ id: 'g', members, incomplete: [] }], unobserved: { humans: [], factions: unobserved, wars: [] },
      countryIdentities: [{ tag: 'AAA', identities: [{ firstHour: 0, lastHour: 100, slot: 1, countryId: 1 }] }] }) as unknown as CountryGroupReply;
  const group = (membership: 'hour' | 'end' | 'ever') => ({ kind: 'group', group: { type: 'faction', id: 'g' }, membership, aggregate: false }) as CountrySelection;
  const whole = [{ firstHour: 0, lastHour: 100 }];
  for (const membership of ['hour', 'end', 'ever'] as const) {
    assert.equal(ledgerCountries(group(membership), [], groups([], whole), 0, 100).reason, 'This group’s membership was not recorded.', membership);
  }
  // Recorded throughout with nobody in it: proven empty.
  assert.equal(ledgerCountries(group('hour'), [], groups([], []), 0, 100).reason, 'This group has no members in this campaign.');
  assert.equal(ledgerCountries(group('end'), [], groups([], []), 0, 100).reason, 'This group has no members at the end of this campaign.');
  // Members only inside unrecorded hours are unknown too.
  assert.equal(ledgerCountries(group('hour'), [], groups([{ tag: 'AAA', firstHour: 10, lastHour: 20 }], [{ firstHour: 0, lastHour: 50 }]), 0, 100).reason,
    'This group’s membership was not recorded.');
  // A recorded member is asked over its own hours.
  assert.deepEqual(ledgerCountries(group('hour'), [], groups([{ tag: 'AAA', firstHour: 10, lastHour: 20 }], []), 0, 100).countries?.members,
    [{ tag: 'AAA', firstHour: 10, lastHour: 20 }]);
});
