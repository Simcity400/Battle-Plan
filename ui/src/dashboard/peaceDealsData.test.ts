import test from 'node:test';
import assert from 'node:assert/strict';
import { validatePeaceDealsReply } from './peaceDealsData.ts';


const holder = (slot: number, tag: string, hour: number) => ({ slot, countryId: 10_000 + slot, tag, hour });
function reply(release: Record<string, unknown>, released: unknown = true) {
  const state = { state: 3, fromCountry: holder(1, 'country/1', 100), toCountry: holder(3, 'country/3', 101), dates: { beforeHour: 100, afterHour: 101 },
    victoryPoints: '5', victoryPointsReadHour: 101, ...(released === undefined ? {} : { released }) };
  const graph = { nodes: [{ id: 'source:1:10001', country: state.fromCountry, side: 'source' }, { id: 'target:3:10003', country: state.toCountry, side: 'target' }],
    links: [{ id: '14:source:1:10001target:3:10003', source: 'source:1:10001', target: 'target:3:10003', stateCount: 1, victoryPoints: '5', states: [state] }] };
  const item = { derived: true, dates: { beforeHour: 100, afterHour: 101 }, winner: null, action: { kind: 'countryReleased', key: null, name: null },
    change: { kind: 'countryReleased', country: 'country/3', from: ['country/1'], states: [3], ...release } };
  const deal = { id: 'outcome:101:0', derived: true, observation: 'conferenceNotRecorded', conferenceId: null, warName: null, firstObservedHour: 101, lastObservedHour: 101,
    currentTurn: null, finalized: null, firstFinalizedHour: null, endHour: 101, endIsBy: true, firstAbsentHour: null, participantsDerived: true, winners: [], losers: [],
    items: [item], captured: null, captureUnavailable: null, transfers: graph, limitations: [] };
  const spans = Object.fromEntries(['settlements', 'conferences', 'ownership', 'wars', 'subjects', 'governments', 'existence', 'victoryPoints'].map(key => [key, []]));
  return { branchId: '1', derived: true, deals: [deal], transfers: graph, unobserved: spans, warnings: [] };
}
const transfers = [{ state: 3, fromCountry: holder(1, 'country/1', 100), victoryPoints: '5', victoryPointsReadHour: 101 }];

test('released states carry their holder and each former owner, and an older bridge reply still opens', () => {
  const current = validatePeaceDealsReply(reply({ toCountry: holder(3, 'country/3', 101), transfers }));
  assert.equal(current.transfers.links[0].states[0].released, true);
  assert.doesNotThrow(() => validatePeaceDealsReply(reply({}, undefined)));
  for (const malformed of [
    reply({ toCountry: holder(3, 'country/3', 101) }),
    reply({ transfers }),
    reply({ toCountry: holder(3, 'country/4', 101), transfers }),
    reply({ toCountry: holder(3, 'country/3', 101), transfers: [{ ...transfers[0], fromCountry: holder(2, 'country/2', 100) }] }),
    reply({ toCountry: holder(3, 'country/3', 101), transfers: [{ ...transfers[0], state: 4 }] }),
    reply({ toCountry: holder(3, 'country/3', 101), transfers }, 'yes'),
  ]) assert.throws(() => validatePeaceDealsReply(malformed), /malformed/);
});
