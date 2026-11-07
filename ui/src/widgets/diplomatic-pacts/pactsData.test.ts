import assert from 'node:assert/strict';
import test from 'node:test';
import { agreementKinds, agreementLanes, agreementsOf, pactSince, pactState } from './pactsData.ts';
import type { DiplomacyCountry, DiplomacyPact as Pact } from '../../generated/dashboard';

const owner: DiplomacyCountry = { tag: 'A', slot: 1, countryId: 7 };
const pact = (id: string, kind: string, firstSeen: number, lastSeen: number, values: Partial<Pact> = {}): Pact => ({ id, kind, owner: 'A', target: 'B', first: 'A', second: 'B',
  startHour: null, endHour: null, cancelled: null, retired: null, firstSeen, lastSeen, presentAtEnd: false, endedBy: false,
  identities: [owner], rawCountries: [], ordinal: 0, cancelRaw: 0, retiredRaw: 0, readHour: firstSeen, goneBy: null, ...values });

test('a pact is gone only from the first read of its owner that no longer lists it', () => {
  // The owner's reads at 0, 24, 48 and 72: the first after its last listing at 24 is 48.
  const listed = pact('1', 'guarantee', 0, 24, { goneBy: 48 });
  assert.deepEqual(pactState(listed, 24), { state: 'present' });
  assert.deepEqual(pactState(listed, 30), { state: 'unknown', since: 24 }, 'between the last listing read and the next read nothing is known');
  assert.deepEqual(pactState(listed, 48), { state: 'gone', by: 48 }, 'dated by the first missing read, not the hour after the last listing');
  assert.deepEqual(pactState(pact('1', 'guarantee', 0, 24), 200), { state: 'unknown', since: 24 }, 'coverage loss is not disappearance');
  assert.deepEqual(pactState(pact('2', 'mod_access', 30, 100, { presentAtEnd: true }), 150), { state: 'present' }, 'still listed at the last read');
  // A gap ended this episode at 24; the owner's recovery read at 48 lists the pact again in a new episode.
  assert.deepEqual(pactState(pact('3', 'guarantee', 0, 24, { endedBy: true }), 48), { state: 'unknown', since: 24 }, 'a suspended episode is not gone');
});

test('agreements are those of the chosen country, one per relation, with offers turned round and kinds shown elsewhere left out', () => {
  // The owner's reads at 0, 50, 100, 300 and 400.
  const pacts = [
    pact('1', 'guarantee', 10, 50, { goneBy: 100 }),
    // One relation listed in two episodes: its stored start names it, so they join.
    pact('2', 'military_access', 30, 60, { startHour: 25 }), pact('3', 'military_access', 90, 100, { startHour: 25, presentAtEnd: true }),
    // An offered access is held by the second country.
    pact('4', 'offer_military_access', 40, 100, { first: 'C', second: 'A' }),
    pact('5', 'lend_lease', 10, 100), pact('6', 'improve_relation', 10, 100),
    pact('7', 'guarantee', 200, 300, { goneBy: 400 }), pact('8', 'guarantee', 10, 100, { first: 'C', second: 'D' }),
  ];
  const at = (hour: number) => agreementsOf(pacts, 'A', hour, null);
  assert.deepEqual(at(40).map(item => [item.kind, item.outgoing, item.partner]).sort(), [['guarantee', true, 'B'], ['military_access', true, 'B'], ['military_access', true, 'C']],
    'a later listing never shows early, and relations between other countries never show');
  const access = at(120).find(item => item.kind === 'military_access' && item.partner === 'B')!;
  assert.deepEqual([access.since, access.sinceBound, access.lastSeen, access.state], [25, false, 100, { state: 'present' }]);
  assert.equal(at(120).filter(item => item.kind === 'military_access' && item.partner === 'B').length, 1);
  assert.deepEqual(at(120).find(item => item.kind === 'guarantee')!.state, { state: 'gone', by: 100 });
  assert.deepEqual(agreementsOf(pacts, 'A', 400, new Set(['guarantee'])).map(item => item.pact.id).sort(), ['1', '7']);
  assert.equal(pactSince(pacts[0]), 10, 'without a stored start, the first read bounds it');
  assert.deepEqual(agreementKinds([{ kind: 'offer_military_access', name: 'Offer access', count: 1, reason: null }, { kind: 'lend_lease', name: null, count: 1, reason: null },
    { kind: 'mod_pact', name: 'Mod pact', count: 1, reason: null }] as never), [{ kind: 'military_access', name: 'Military access' }, { kind: 'mod_pact', name: 'Mod pact' }]);
});

test('lanes are a section per kind, split by side, with ended agreements faded or hidden', () => {
  const pacts = [pact('1', 'embargo', 10, 50, { first: 'B', second: 'A', goneBy: 100 }), pact('2', 'embargo', 10, 100, { presentAtEnd: true }),
    pact('3', 'non_aggression_pact', 10, 100, { presentAtEnd: true, startHour: 5, endHour: 900 }), pact('4', 'mod_pact', 10, 100, { presentAtEnd: true })];
  const agreements = agreementsOf(pacts, 'A', 100, null);
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

test('a pact listed at the last prepared read is not in force beyond it while later hours are still being prepared', () => {
  const listed = pact('1', 'non_aggression_pact', 10, 100, { presentAtEnd: true });
  assert.deepEqual(pactState(listed, 100, 100), { state: 'present' });
  assert.deepEqual(pactState(listed, 150, 100), { state: 'preparing', since: 100 });
  assert.deepEqual(pactState(listed, 150), { state: 'present' }, 'once prepared, still listed at the last read is in force');
  const agreements = agreementsOf([listed], 'A', 150, null, 100);
  const text = { name: 'Alpha', country: (tag: string | null) => ({ name: `Country ${tag}` }), kindName: (kind: string) => kind, colors: ['#112233'] };
  const { lanes } = agreementLanes(agreements, false, 100, text);
  assert.equal(lanes[0].identity?.label, 'Non-aggression pacts · 0 in force', 'not counted in force at an hour still being prepared');
  assert.equal(lanes[1].spans[0].lastHour, 100, 'its bar runs only through the prepared hours');
  assert.equal(lanes[1].muted, false, 'nor is it shown ended');
});
