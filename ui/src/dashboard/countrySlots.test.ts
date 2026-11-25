import test from 'node:test';
import assert from 'node:assert/strict';
import { answersKey, countryFromKey, countryKey, isSlotKey, SLOT_MARK } from './countryKey.ts';
import { SlotTags } from './countrySlots.ts';
import { defaultCountryKey, selectedCountryKeys } from './countrySelection.ts';
import type { BoundTrack } from './model';

const identities = new SlotTags({ countries: [
  { tag: 'GER', identities: [{ firstHour: 100, lastHour: 109, slot: 5, countryId: 50 }, { firstHour: 110, lastHour: 119, slot: 9, countryId: 90 }], unresolved: [] },
  { tag: 'DEU', identities: [{ firstHour: 110, lastHour: 119, slot: 5, countryId: 50 }], unresolved: [] },
] });

test('a slot names the tag its country goes by at each hour, never the tag another country takes', () => {
  assert.equal(identities.tagAt(5, 105), 'GER');
  assert.equal(identities.tagAt(5, 115), 'DEU');
  assert.equal(identities.tagAt(9, 115), 'GER');
  assert.equal(identities.tagAt(5, 125), null, 'an hour the country holds no tag is not recorded');
  assert.equal(identities.nameAt(5, 125), 'DEU', 'a label names it by the nearest tag it held');
  assert.equal(identities.slotOf('GER', 105), 5);
  assert.equal(identities.slotOf('GER', 115), 9, 'a pick of a tag is the country holding it at that hour');
  assert.deepEqual(identities.tagRuns(5).map(run => run.tag), ['GER', 'DEU']);
  assert.deepEqual(identities.runNear(5, 115), { tag: 'DEU', firstHour: 110, lastHour: 119 }, 'the run of the tag it goes by then');
  assert.deepEqual(identities.runNear(5, 99), { tag: 'GER', firstHour: 100, lastHour: 109 }, 'else the nearest run it held');
  assert.equal(identities.runNear(7, 115), null);
});

test('two holders of one slot at once: the run at an hour is the one of the lowest country id, whichever started first', () => {
  const shared = new SlotTags({ countries: [
    { tag: 'OLD', identities: [{ firstHour: 0, lastHour: 200, slot: 5, countryId: 90 }], unresolved: [] },
    { tag: 'NEW', identities: [{ firstHour: 150, lastHour: 200, slot: 5, countryId: 40 }], unresolved: [] },
  ] });
  assert.deepEqual(shared.runAt(5, 160), { tag: 'NEW', firstHour: 150, lastHour: 200 }, 'the tag the slot goes by then, and its hours');
  assert.equal(shared.tagAt(5, 160), 'NEW');
  assert.deepEqual(shared.runAt(5, 100), { tag: 'OLD', firstHour: 0, lastHour: 200 }, 'before the lower id holds it');
  assert.equal(shared.runAt(5, 201), null, 'an hour it holds no tag');
  assert.deepEqual(
    shared.tagRuns(5).map(({ tag, firstHour, lastHour }) => ({ tag, firstHour, lastHour })),
    [{ tag: 'OLD', firstHour: 0, lastHour: 149 }, { tag: 'NEW', firstHour: 150, lastHour: 200 }],
    'overlapping slot runs split so the lowest countryId wins at every hour',
  );
});

test('tag existence runs retain unresolved spans so existence matches the board query', () => {
  const withUnresolved = new SlotTags({ countries: [
    { tag: 'GER', identities: [{ firstHour: 0, lastHour: 10, slot: 5, countryId: 50 }], unresolved: [{ firstHour: 11, lastHour: 20 }] },
  ] });
  assert.deepEqual(withUnresolved.tagExistenceRuns('GER'), [
    { tag: 'GER', firstHour: 0, lastHour: 10 },
    { tag: 'GER', firstHour: 11, lastHour: 20, unresolved: true },
  ]);
});

test('a country key names a slot by a form no tag can spell, and reads back as the country it names', () => {
  assert.equal(countryKey(5), `${SLOT_MARK}5`);
  assert.notEqual(countryKey(5), countryKey('5'), 'a mod tag spelled as a number is never a slot');
  assert.equal(countryKey('GER'), 'GER');
  assert.equal(countryFromKey(countryKey(5)), 5);
  assert.equal(countryFromKey('5'), '5');
  assert.equal(countryFromKey('GER'), 'GER');
  assert.equal(countryFromKey(`${SLOT_MARK}x`), `${SLOT_MARK}x`);
  assert.ok(isSlotKey(countryKey(1499)) && !isSlotKey('1499'));
});

test('a single-country reply is taken only for the country its request named', () => {
  assert.ok(answersKey(5, countryKey(5)), 'a reply asked by slot answers that slot, whatever tag it went by');
  assert.ok(!answersKey(9, countryKey(5)), 'a reply asked for the country before a switch is never taken for the new one');
  assert.ok(!answersKey('GER', countryKey(5)) && !answersKey(5, 'GER'), 'a tag and a slot never answer each other');
  assert.ok(answersKey('GER', 'GER') && !answersKey('ITA', 'GER'));
  assert.ok(!answersKey(undefined, countryKey(5)) && !answersKey(5, null));
});

test('the dashboard selects its country by slot, and a widget pinning a tag keeps that tag', () => {
  const source = { metadata: { playerCountryId: '7', countries: [] } } as unknown as BoundTrack['source'];
  assert.equal(defaultCountryKey({ source, selectedSlot: 5 }), countryKey(5));
  assert.equal(defaultCountryKey({ source, selectedSlot: null }), countryKey(7), 'with no choice, the recorded player country');
  assert.equal(defaultCountryKey({ source: { metadata: { playerCountryId: null, countries: [] } } as unknown as BoundTrack['source'] }), null);
  assert.deepEqual(selectedCountryKeys({ kind: 'selected' }, { source, selectedSlot: 5 }), [countryKey(5)]);
  assert.deepEqual(selectedCountryKeys({ kind: 'tags', tags: ['ITA'] }, { source, selectedSlot: 5 }), ['ITA']);
  assert.deepEqual(selectedCountryKeys({ kind: 'all' }, { source, selectedSlot: 5 }), []);
});

test('coalesced identities with known spans report null on unrecorded gap hours and the holder on recorded hours', () => {
  const sparse = new SlotTags({
    known: [{ firstHour: 100, lastHour: 100 }, { firstHour: 110, lastHour: 110 }],
    countries: [{ tag: 'GER', identities: [{ firstHour: 100, lastHour: 110, slot: 5, countryId: 50 }], unresolved: [] }],
  });
  assert.equal(sparse.tagAt(5, 100), 'GER');
  assert.equal(sparse.tagAt(5, 105), null);
  assert.equal(sparse.tagAt(5, 110), 'GER');
  assert.equal(sparse.slotOf('GER', 100), 5);
  assert.equal(sparse.slotOf('GER', 105), null);
  assert.equal(sparse.nameAt(5, 105), 'GER');
});
