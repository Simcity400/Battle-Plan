import { PENDING_ART } from '../../families/art/types.ts';
import assert from 'node:assert/strict';
import test from 'node:test';
import { agreementKinds, agreementLanes, agreementsOf, pactSince, pactState, pactsProof, validatePacts } from './pactsData.ts';
import type { DiplomacyPactsReply, DiplomacyPact as Pact } from '../../generated/dashboard';

const ids = new Map<Pact, string>();
const idOf = (pact: Pact) => ids.get(pact);
const pact = (id: string, kind: string, firstSeen: number, lastSeen: number, values: Partial<Pact> = {}): Pact => {
  const made: Pact = { kind, first: 'A', second: 'B', startHour: null, endHour: null, firstSeen, lastSeen, held: false, endedBy: false, goneBy: null, ...values };
  ids.set(made, id);
  return made;
};

test('a pact’s state is the product’s at the hour asked: held, gone by the first read that no longer lists it, else unknown', () => {
  assert.deepEqual(pactState(pact('1', 'guarantee', 0, 24, { held: true })), { state: 'present' });
  assert.deepEqual(pactState(pact('1', 'guarantee', 0, 24, { goneBy: 48 })), { state: 'gone', by: 48 });
  assert.deepEqual(pactState(pact('1', 'guarantee', 0, 24)), { state: 'unknown', since: 24 }, 'not listed after its last read, and no read since says it is gone');
  assert.deepEqual(pactState(pact('3', 'guarantee', 0, 24, { endedBy: true, goneBy: 48 })), { state: 'unknown', since: 24 }, 'a suspended episode is not gone');
});

test('agreements are those of the chosen country, one per relation, with offers turned round and kinds shown elsewhere left out', () => {
  const pacts = [
    pact('1', 'guarantee', 10, 50, { goneBy: 100 }),
    pact('2', 'military_access', 30, 60, { startHour: 25 }), pact('3', 'military_access', 90, 120, { startHour: 25, held: true }),
    pact('4', 'offer_military_access', 40, 120, { first: 'C', second: 'A', held: true }),
    pact('5', 'lend_lease', 10, 120, { held: true }), pact('6', 'improve_relation', 10, 120, { held: true }),
    pact('8', 'guarantee', 10, 120, { first: 'C', second: 'D', held: true }),
  ];
  const all = agreementsOf(pacts, 'A', null);
  assert.deepEqual(all.map(item => [item.kind, item.outgoing, item.partner]).sort(), [['guarantee', true, 'B'], ['military_access', true, 'B'], ['military_access', true, 'C']],
    'relations between other countries and kinds shown elsewhere never show');
  const access = all.find(item => item.kind === 'military_access' && item.partner === 'B')!;
  assert.deepEqual([access.since, access.sinceBound, access.lastSeen, access.state], [25, false, 120, { state: 'present' }]);
  assert.deepEqual(all.find(item => item.kind === 'guarantee')!.state, { state: 'gone', by: 100 });
  assert.deepEqual(agreementsOf(pacts, 'A', new Set(['guarantee'])).map(item => idOf(item.pact)), ['1']);
  assert.equal(pactSince(pacts[0]!), 10, 'without a stored start, the first read bounds it');
  assert.deepEqual(agreementKinds([{ kind: 'offer_military_access', name: 'Offer access', count: 1, reason: null }, { kind: 'lend_lease', name: null, count: 1, reason: null },
    { kind: 'mod_pact', name: 'Mod pact', count: 1, reason: null }] as never), [{ kind: 'military_access', name: 'Military access' }, { kind: 'mod_pact', name: 'Mod pact' }]);
});

test('lanes are a section per kind, split by side, with ended agreements shown or hidden', () => {
  const pacts = [pact('1', 'embargo', 10, 50, { first: 'B', second: 'A', goneBy: 100 }), pact('2', 'embargo', 10, 100, { held: true }),
    pact('3', 'non_aggression_pact', 10, 100, { held: true, startHour: 5, endHour: 900 }), pact('4', 'mod_pact', 10, 100, { held: true })];
  const agreements = agreementsOf(pacts, 'A', null);
  const text = { name: 'Alpha', country: (tag: string) => ({ name: `Country ${tag}`, flag: PENDING_ART }), kindName: (kind: string) => kind === 'mod_pact' ? 'Mod pact' : kind,
    colour: () => '#112233' };
  const lanes = agreementLanes(agreements, true, 110, text);
  assert.deepEqual(lanes.map(lane => lane.subtitle ? `${lane.label} · ${lane.subtitle}` : lane.label), ['Non-aggression pacts · 1 in force', 'Country B', 'Trade embargoes · 1 in force',
    'Embargoed by', 'Country B', 'Alpha embargoes', 'Country B', 'Mod pact · 1 in force', 'Alpha → Partner', 'Country B'], 'engine kinds first, embargoes received first, a mod kind by its game name');
  const endedLane = lanes.find(lane => lane.meta && idOf((lane.meta as { agreement: { pact: Pact } }).agreement.pact) === '1')!;
  assert.deepEqual([endedLane.spans[0]!.open, endedLane.spans[0]!.lastHour, endedLane.spans[0]!.endUnseen], [false, 50, true], 'gone by a later read is an unseen end');
  const live = lanes.find(lane => lane.meta && idOf((lane.meta as { agreement: { pact: Pact } }).agreement.pact) === '2')!;
  assert.deepEqual([live.spans[0]!.open, live.spans[0]!.lastHour], [true, 110], 'held runs on to the displayed hour inside the reply’s proof');
  assert.ok(!agreementLanes(agreements, false, 110, text).some(lane => lane.meta && idOf((lane.meta as { agreement: { pact: Pact } }).agreement.pact) === '1'), 'Ended hidden');
});

test('an agreements reply holds nothing after its hour and proves up to its next change', () => {
  const reply: DiplomacyPactsReply = { hour: 20, nextChange: 30, kinds: [{ kind: 'guarantee', count: 1, name: null, reason: 'not read' }], pacts: [pact('9', 'guarantee', 1, 20, { held: true })], unobserved: [], warnings: [] };
  assert.equal(validatePacts(reply), reply);
  assert.throws(() => validatePacts({ ...reply, pacts: [{ ...reply.pacts[0], lastSeen: 21 }] }), /malformed/, 'a pact seen after the hour asked is refused');
  assert.throws(() => validatePacts({ ...reply, pacts: [{ ...reply.pacts[0], firstSeen: 'soon' }] }), /malformed/);
  assert.throws(() => validatePacts({ pacts: [] }), /malformed/);
  assert.deepEqual(pactsProof(reply, [0, 20], 25), [20, 29]);
  assert.equal(pactsProof(reply, [0, 20], 30), null);
});
