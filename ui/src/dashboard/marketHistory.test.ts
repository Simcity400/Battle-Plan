import test from 'node:test';
import assert from 'node:assert/strict';
import { identityTagAt, partiesOf, partyTagsAt } from './marketHistory.ts';
import type { PreparedPrefixFields } from './preparedPrefix';

test('a contract drill follows both parties through their tag changes', () => {
  const seller = { id: '1', tags: [{ hour: 0, tag: 'AAA' }] }, buyer = { id: '2', tags: [{ hour: 0, tag: 'CCC' }] };
  const current = { '1': [{ hour: 0, tag: 'AAA' }, { hour: 50, tag: 'BBB' }], '2': [{ hour: 0, tag: 'CCC' }, { hour: 60, tag: 'DDD' }] };
  assert.deepEqual(partyTagsAt([seller, buyer], current, 10), ['AAA', 'CCC']);
  assert.deepEqual(partyTagsAt([seller, buyer], current, 70), ['BBB', 'DDD'], 'never the tags the row had when picked');
  assert.deepEqual(partyTagsAt([seller, buyer], undefined, 70), ['AAA', 'CCC'], 'the tags kept at the pick until the reply has them');
  assert.deepEqual(partyTagsAt([seller, { id: '3', tags: [{ hour: 80, tag: 'EEE' }] }], current, 70), ['BBB'], 'one recorded party still finds the contract');
  assert.equal(partyTagsAt([{ id: '3', tags: [{ hour: 80, tag: 'EEE' }] }], current, 70), null, 'no party recorded then');
});

test('a pick keeps each identified party by its holder, never an unidentified one', () => {
  const holders = { '1': [{ hour: 0, tag: 'AAA' }], '2': [{ hour: 0, tag: 'CCC' }] };
  assert.deepEqual(partiesOf(holders, ['1', null, '2']), [{ id: '1', tags: holders['1'] }, { id: '2', tags: holders['2'] }]);
  assert.deepEqual(partiesOf(holders, ['9']), [{ id: '9', tags: [] }], 'a holder without tags is still the pick, tagged only once recorded');
});

test('a picked party follows the identity history only where it is exact, else what the pages read', () => {
  // Holder 1 is SEL until 29 and NSL from 30; holder 2 is BUY until 34 and NBY from 35.
  const identities = [
    { tag: 'SEL', identities: [{ firstHour: 0, lastHour: 29, slot: 1, countryId: 10 }] },
    { tag: 'NSL', identities: [{ firstHour: 30, lastHour: 50, slot: 1, countryId: 10 }] },
    { tag: 'BUY', identities: [{ firstHour: 0, lastHour: 34, slot: 2, countryId: 20 }] },
    { tag: 'NBY', identities: [{ firstHour: 35, lastHour: 50, slot: 2, countryId: 20 }] },
  ];
  const picked = partiesOf({ '1': [{ hour: 0, tag: 'SEL' }], '2': [{ hour: 0, tag: 'BUY' }] }, ['1', '2']);
  const whole: PreparedPrefixFields = { coverage: { firstHour: 0, lastHour: 50, complete: true }, pending: [] };
  const exactIn = (prefix: PreparedPrefixFields | null) => (id: string, hour: number) => identityTagAt(identities, prefix, id, hour);
  assert.deepEqual(partyTagsAt(picked, undefined, 38, exactIn(whole)), ['NSL', 'NBY'], 'no loaded page names either party');
  assert.deepEqual(partyTagsAt(picked, undefined, 10, exactIn(whole)), ['SEL', 'BUY']);
  // Prepared through 31, the tail sampled: exact NSL at 31, but 38 falls back to the pages.
  const partial: PreparedPrefixFields = { coverage: { firstHour: 0, lastHour: 31, complete: false }, pending: [{ firstHour: 32, lastHour: 50 }] };
  const pages = { '1': [{ hour: 0, tag: 'SEL' }, { hour: 30, tag: 'NSL' }], '2': [{ hour: 0, tag: 'BUY' }, { hour: 35, tag: 'NBY' }] };
  assert.deepEqual(partyTagsAt(picked, pages, 38, exactIn(partial)), ['NSL', 'NBY'], 'a sampled tail never displaces what the pages read');
  assert.equal(identityTagAt(identities, partial, '2', 38), null);
  assert.equal(identityTagAt(identities, partial, '1', 31), 'NSL');
  // An approximate span is not exact either, and a run never reaches past its last hour.
  assert.equal(identityTagAt(identities, { ...whole, approximate: [{ firstHour: 30, lastHour: 40 }] }, '1', 35), null);
  assert.equal(identityTagAt(identities, whole, '1', 60), null);
});
