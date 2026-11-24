import assert from 'node:assert/strict';
import test from 'node:test';
import { agreementKinds, agreementLanes, agreementsOf, changeHours, pactSince, pactState, validatePacts } from './pactsData.ts';
import { changesBy } from '../../charts/timeline/runState.ts';
import type { DiplomacyPact as Pact } from '../../generated/dashboard';

const ids = new Map<Pact, string>();
const idOf = (pact: Pact) => ids.get(pact);
const pact = (id: string, kind: string, firstSeen: number, lastSeen: number, values: Partial<Pact> = {}): Pact => {
  const made: Pact = { kind, first: 'A', second: 'B', startHour: null, endHour: null, firstSeen, lastSeen, presentAtEnd: false, endedBy: false, goneBy: null, ...values };
  ids.set(made, id);
  return made;
};

test('a pact is gone only from the first read of its owner that no longer lists it', () => {
  const listed = pact('1', 'guarantee', 0, 24, { goneBy: 48 });
  assert.deepEqual(pactState(listed, 24), { state: 'present' });
  assert.deepEqual(pactState(listed, 30), { state: 'unknown', since: 24 }, 'between the last listing read and the next read nothing is known');
  assert.deepEqual(pactState(listed, 48), { state: 'gone', by: 48 }, 'dated by the first missing read, not the hour after the last listing');
  assert.deepEqual(pactState(pact('1', 'guarantee', 0, 24), 200), { state: 'unknown', since: 24 }, 'coverage loss is not disappearance');
  assert.deepEqual(pactState(pact('2', 'mod_access', 30, 100, { presentAtEnd: true }), 150), { state: 'present' }, 'still listed at the last read');
  assert.deepEqual(pactState(pact('3', 'guarantee', 0, 24, { endedBy: true }), 48), { state: 'unknown', since: 24 }, 'a suspended episode is not gone');
});

test('agreements are those of the chosen country, one per relation, with offers turned round and kinds shown elsewhere left out', () => {
  const pacts = [
    pact('1', 'guarantee', 10, 50, { goneBy: 100 }),
    pact('2', 'military_access', 30, 60, { startHour: 25 }), pact('3', 'military_access', 90, 100, { startHour: 25, presentAtEnd: true }),
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
  assert.deepEqual(agreementsOf(pacts, 'A', 400, new Set(['guarantee'])).map(item => idOf(item.pact)).sort(), ['1', '7']);
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
  const ended = lanes.find(lane => (info.get(lane.id) && idOf(info.get(lane.id)!.agreement.pact)) === '1')!;
  assert.equal(ended.muted, true);
  assert.equal(ended.spans[0].color, '#11223366');
  assert.equal(info.get(lanes[1].id)!.agreement.expires, 900);
  assert.equal(lanes.find(lane => (info.get(lane.id) && idOf(info.get(lane.id)!.agreement.pact)) === '2')!.spans[0].lastHour, 1000, 'still listed at the last read runs to the branch end');
  assert.ok(!agreementLanes(agreements, false, 1000, text).lanes.some(lane => (info.get(lane.id) && idOf(info.get(lane.id)!.agreement.pact)) === '1'), 'Ended hidden');
});


test('an agreements reply of another shape is refused, never read wrongly', () => {
  const reply = { kinds: [{ kind: 'guarantee', count: 1, name: null, reason: 'not read' }], pacts: [pact('9', 'guarantee', 1, 2)], unobserved: [], warnings: [] };
  assert.equal(validatePacts(reply), reply);
  assert.throws(() => validatePacts({ ...reply, pacts: [{ ...reply.pacts[0], firstSeen: 'soon' }] }), /malformed/);
  assert.throws(() => validatePacts({ pacts: [] }), /malformed/);
});

test('the timeline renders only where an agreement is listed, starts, expires or is gone, or a stretch not recorded changes', () => {
  const hours = changeHours({ pacts: [pact('1', 'guarantee', 0, 24, { goneBy: 48, startHour: -100, endHour: 500 })], unobserved: [{ firstHour: 60, lastHour: 70 }] });
  assert.deepEqual(hours, [-100, 0, 25, 48, 60, 71, 500]);
  assert.equal(changesBy(hours, 1), changesBy(hours, 24), 'listed and unchanged: no render however the head moves');
  assert.notEqual(changesBy(hours, 24), changesBy(hours, 25));
});
