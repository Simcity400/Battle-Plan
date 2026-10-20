import test from 'node:test';
import assert from 'node:assert/strict';
import type { CountryGroupReply } from '../generated/dashboard';
import type { CountrySelection } from './model';
import { groupReasonAt, preparedAt } from './economyHistory.ts';

const groups = (members: { tag: string; firstHour: number; lastHour: number }[], unobserved: { firstHour: number; lastHour: number }[] = []) =>
  ({ humans: [], wars: [], countryIdentities: [], factions: [{ id: 'g', members, incomplete: [] }],
    unobserved: { humans: [], factions: unobserved, wars: [] } }) as unknown as CountryGroupReply;
const faction = (membership: 'hour' | 'end' | 'ever'): CountrySelection =>
  ({ kind: 'group', group: { type: 'faction', id: 'g' }, membership, aggregate: false }) as CountrySelection;

test('a group with members at the hour leaves its rows to the countries\' own evidence', () => {
  assert.equal(groupReasonAt(faction('hour'), groups([{ tag: 'AAA', firstHour: 0, lastHour: 50 }]), 10, 50), null);
  // Other selections, and groups still loading, are not judged here.
  assert.equal(groupReasonAt({ kind: 'all' } as CountrySelection, undefined, 10, 50), null);
  assert.equal(groupReasonAt(faction('hour'), undefined, 10, 50), null);
});

test('a group with no members, or an unrecorded roster, is never shown as no activity', () => {
  // Members only later: empty at hour 10, proven.
  assert.deepEqual(groupReasonAt(faction('hour'), groups([{ tag: 'AAA', firstHour: 30, lastHour: 50 }]), 10, 50),
    { kind: 'empty', message: 'This group has no members at this date.' });
  // The roster was not observed at hour 10: unknown, not empty.
  assert.equal(groupReasonAt(faction('hour'), groups([{ tag: 'AAA', firstHour: 0, lastHour: 50 }], [{ firstHour: 5, lastHour: 15 }]), 10, 50)?.kind,
    'unavailable');
  assert.equal(groupReasonAt(faction('ever'), groups([]), 10, 50)?.message, 'This group has no members in this campaign.');
});

test('an hour past the prepared prefix, or inside a pending span, is not prepared', () => {
  const reply = { lastHour: 40, coverage: { firstHour: 0, lastHour: 60, complete: false }, pending: [{ firstHour: 20, lastHour: 30 }] };
  assert.equal(preparedAt(reply, 10), true);
  assert.equal(preparedAt(reply, 25), false);
  assert.equal(preparedAt(reply, 45), false);
  assert.equal(preparedAt({ ...reply, lastHour: null }, 10), false);
});

test('a group still being prepared at the hour its members are counted has no reason yet; a read hour and a final reply keep theirs', () => {
  const empty = { firstHour: 0, lastHour: 160 };
  const prefix = { ...groups([{ tag: 'AAA', firstHour: 0, lastHour: 100 }]), coverage: { ...empty, complete: false },
    pending: [{ firstHour: 161, lastHour: 200 }] } as CountryGroupReply;
  // AAA left at 100: no member at 150 (read) is proven, at 170 (pending) it is not.
  assert.equal(groupReasonAt(faction('hour'), prefix, 150, 200)?.message, 'This group has no members at this date.');
  assert.equal(groupReasonAt(faction('hour'), prefix, 170, 200), null);
  // The end and the whole branch wait for the hours they count.
  assert.equal(groupReasonAt(faction('end'), prefix, 150, 200), null);
  assert.equal(groupReasonAt(faction('ever'), { ...prefix, factions: [{ id: 'g', members: [], incomplete: [] }] } as unknown as CountryGroupReply, 150, 200), null);
  const final = groups([{ tag: 'AAA', firstHour: 0, lastHour: 100 }]);
  assert.equal(groupReasonAt(faction('hour'), final, 170, 200)?.message, 'This group has no members at this date.');
  assert.equal(groupReasonAt(faction('end'), final, 150, 200)?.message, 'This group has no members at the end of this campaign.');
});
