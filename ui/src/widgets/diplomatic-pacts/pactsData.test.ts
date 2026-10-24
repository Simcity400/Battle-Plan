import assert from 'node:assert/strict';
import test from 'node:test';
import { agreementKinds, agreementLanes, agreementsOf, ownerReads, pactSince, pactState } from './pactsData.ts';
import type { DiplomacyCountry, DiplomacyPact as Pact, DiplomacyReadRun } from '../../generated/dashboard';

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

test('agreements are those of the chosen country, one per relation, with offers turned round and kinds shown elsewhere left out', () => {
  const reads = ownerReads([read(0), read(50), read(100), read(300), read(400)]);
  const pacts = [
    pact('1', 'guarantee', 10, 50),
    // One relation listed in two episodes: its stored start names it, so they join.
    pact('2', 'military_access', 30, 60, { startHour: 25 }), pact('3', 'military_access', 90, 100, { startHour: 25, presentAtEnd: true }),
    // An offered access is held by the second country.
    pact('4', 'offer_military_access', 40, 100, { first: 'C', second: 'A' }),
    pact('5', 'lend_lease', 10, 100), pact('6', 'improve_relation', 10, 100),
    pact('7', 'guarantee', 200, 300), pact('8', 'guarantee', 10, 100, { first: 'C', second: 'D' }),
  ];
  const at = (hour: number) => agreementsOf(pacts, 'A', hour, reads, null);
  assert.deepEqual(at(40).map(item => [item.kind, item.outgoing, item.partner]).sort(), [['guarantee', true, 'B'], ['military_access', true, 'B'], ['military_access', true, 'C']],
    'a later listing never shows early, and relations between other countries never show');
  const access = at(120).find(item => item.kind === 'military_access' && item.partner === 'B')!;
  assert.deepEqual([access.since, access.sinceBound, access.lastSeen, access.state], [25, false, 100, { state: 'present' }]);
  assert.equal(at(120).filter(item => item.kind === 'military_access' && item.partner === 'B').length, 1);
  assert.deepEqual(at(120).find(item => item.kind === 'guarantee')!.state, { state: 'gone', by: 100 });
  assert.deepEqual(agreementsOf(pacts, 'A', 400, reads, new Set(['guarantee'])).map(item => item.pact.id).sort(), ['1', '7']);
  assert.equal(pactSince(pacts[0]), 10, 'without a stored start, the first read bounds it');
  assert.deepEqual(agreementKinds([{ kind: 'offer_military_access', name: 'Offer access', count: 1, reason: null }, { kind: 'lend_lease', name: null, count: 1, reason: null },
    { kind: 'mod_pact', name: 'Mod pact', count: 1, reason: null }] as never), [{ kind: 'military_access', name: 'Military access' }, { kind: 'mod_pact', name: 'Mod pact' }]);
});

test('lanes are a section per kind, split by side, with ended agreements faded or hidden', () => {
  const reads = ownerReads([read(0), read(50), read(100)]);
  const pacts = [pact('1', 'embargo', 10, 50, { first: 'B', second: 'A' }), pact('2', 'embargo', 10, 100, { presentAtEnd: true }),
    pact('3', 'non_aggression_pact', 10, 100, { presentAtEnd: true, startHour: 5, endHour: 900 }), pact('4', 'mod_pact', 10, 100, { presentAtEnd: true })];
  const agreements = agreementsOf(pacts, 'A', 100, reads, null);
  const text = { name: 'Alpha', country: (tag: string | null) => ({ name: `Country ${tag}` }), kindName: (kind: string) => kind === 'mod_pact' ? 'Mod pact' : kind, colors: ['#112233'] };
  const { lanes, info } = agreementLanes(agreements, true, 1000, text);
  assert.deepEqual(lanes.map(lane => lane.identity?.label), ['Non-aggression pacts · 1 in force', 'Country B', 'Trade embargoes · 1 in force', 'Embargoed by', 'Country B',
    'Alpha embargoes', 'Country B', 'Mod pact · 1 in force', 'Alpha → partner', 'Country B'], 'engine kinds first, embargoes received first, a mod kind by its game name');
  const ended = lanes.find(lane => info.get(lane.id)?.agreement.pact.id === '1')!;
  assert.equal(ended.muted, true);
  assert.equal(ended.spans[0].color, '#11223366');
  assert.equal(info.get(lanes[1].id)!.agreement.expires, 900);
  assert.equal(lanes.find(lane => info.get(lane.id)?.agreement.pact.id === '2')!.spans[0].lastHour, 1000, 'still listed at the last read runs to the branch end');
  assert.ok(!agreementLanes(agreements, false, 1000, text).lanes.some(lane => info.get(lane.id)?.agreement.pact.id === '1'), 'Ended hidden');
});
