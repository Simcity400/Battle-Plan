import assert from 'node:assert/strict';
import test from 'node:test';
import { days, ownerReads, pactSince, pactState, proposalHour, proposalState, shownPacts, shownProposals } from './pactsData.ts';
import type { DiplomacyCountry, DiplomacyPact as Pact, DiplomacyProposal as Proposal, DiplomacyReadRun } from '../../generated/dashboard';

const owner: DiplomacyCountry = { tag: 'A', slot: 1, countryId: 7 };
const pact = (id: string, kind: string, firstSeen: number, lastSeen: number, values: Partial<Pact> = {}): Pact => ({ id, kind, owner: 'A', target: 'B', first: 'A', second: 'B',
  startHour: null, endHour: null, cancelled: null, retired: null, firstSeen, lastSeen, presentAtEnd: false, endedBy: false,
  identities: [owner], rawCountries: [], ordinal: 0, cancelRaw: 0, retiredRaw: 0, readHour: firstSeen, ...values });
const read = (readHour: number, firstHour = readHour, lastHour = readHour + 23): DiplomacyReadRun => ({ owner, firstHour, lastHour, readHour });

test('a pact is gone only from the first read of its owner that no longer lists it', () => {
  const reads = ownerReads([read(0), read(24), read(48), read(72)]);
  const listed = pact('1', 'guarantee', 0, 24);
  assert.deepEqual(pactState(listed, 24, reads), { state: 'present' });
  assert.deepEqual(pactState(listed, 30, reads), { state: 'unknown', since: 24 }, 'between the last listing read and the next read nothing is known');
  assert.deepEqual(pactState(listed, 48, reads), { state: 'gone', by: 48 }, 'dated by the first missing read, not the hour after the last listing');
  const uncovered = ownerReads([read(0), read(24)]);
  assert.deepEqual(pactState(listed, 200, uncovered), { state: 'unknown', since: 24 }, 'coverage loss is not disappearance');
  assert.deepEqual(pactState(pact('2', 'mod_access', 30, 100, { presentAtEnd: true }), 150, reads), { state: 'present' }, 'still listed at the last read');
  // A gap ended this episode at 24; the owner's recovery read at 48 lists the pact again in a new episode.
  assert.deepEqual(pactState(pact('3', 'guarantee', 0, 24, { endedBy: true }), 48, reads), { state: 'unknown', since: 24 }, 'a suspended episode is not gone');
});

test('pacts appear at the read that listed them and narrow by kind and the Ended setting', () => {
  const reads = ownerReads([read(0), read(50), read(100), read(300), read(400)]);
  const pacts = [pact('1', 'guarantee', 10, 50), pact('2', 'mod_access', 30, 100, { presentAtEnd: true, startHour: 5 }), pact('3', 'guarantee', 200, 300)];
  assert.deepEqual(shownPacts(pacts, 40, null, true, reads).map(item => item.id), ['1', '2'], 'a later observation never shows early');
  assert.deepEqual(shownPacts(pacts, 120, null, false, reads).map(item => item.id), ['2'], 'Ended hidden keeps only those still listed');
  assert.deepEqual(shownPacts(pacts, 400, new Set(['guarantee']), true, reads).map(item => item.id), ['1', '3']);
  assert.equal(pactSince(pacts[0]), 10, 'without a stored start, the first read bounds it');
  assert.equal(pactSince(pacts[1]), 5);
  assert.equal(days(24, 24 * 5 + 23), 4);
});

test('proposals wait while listed, are gone at the first read without them, and are unknown otherwise', () => {
  const item = (id: string, firstSeen: number, lastSeen: number, goneBy: number | null, presentAtEnd = false): Proposal => ({ id, handle: id, owner: 'B', sender: 'A', recipient: 'B',
    originalSender: 'A', originalRecipient: 'B', identities: [], rawCountries: [], storedHour: firstSeen, lastCommandHour: firstSeen, firstSeen, lastSeen,
    goneBy, goneByBound: false, presentAtEnd, readHour: firstSeen });
  const gone = item('1', 10, 22, 30), open = item('2', 20, 500, null, true), suspended = item('3', 5, 40, null);
  assert.equal(proposalState(gone, 22), 'waiting');
  assert.equal(proposalState(gone, 25), 'unknown');
  assert.equal(proposalState(gone, 30), 'gone');
  assert.equal(proposalState(open, 1000), 'waiting', 'still in the queue at the last read');
  assert.equal(proposalState(suspended, 100), 'unknown', 'an episode a gap suspended is not waiting');
  assert.deepEqual(shownProposals([gone, open, suspended], 100, true).map(p => p.id), ['2'], 'waiting only');
  assert.deepEqual(shownProposals([gone, open, suspended], 15, false).map(p => p.id), ['1', '3']);
  assert.equal(proposalHour(gone), 10);
});
