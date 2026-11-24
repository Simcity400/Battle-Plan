import assert from 'node:assert/strict';
import test from 'node:test';
import type { WarObservation, WarParticipantRun, WarTimeline, WarsReply } from '../../generated/dashboard';
import { isWarsReply, labelObservationAt, observationAt, validateWarsReply } from './warsData.ts';

const participant = (tag: string, side: WarParticipantRun['side'], firstHour = 10, lastHour = 19): WarParticipantRun =>
  ({ tag, side, firstHour, lastHour, joinedBy: true, leftBy: false });
const observation = (firstHour = 10, lastHour = 19, attackersAreViewpoint: boolean | null = true): WarObservation => ({
  firstHour, lastHour, viewpointTag: 'Mod: player', viewpointSide: ['One side'], opposingSide: ['Other side'],
  attackersAreViewpoint, representative: ['One side', 'Other side'], name: { text: 'Recorded name', known: true, reason: null, key: null },
});
const war = (id = '1', firstHour = 10, lastHour = 19): WarTimeline => ({
  id, firstHour, lastHour, startedBy: true, endedBy: false, observations: [observation(firstHour, lastHour)],
  participants: [participant('One side', 'attacker', firstHour, lastHour), participant('Other side', 'defender', firstHour, lastHour)],
  capitulations: [],
});
const reply = (wars: WarTimeline[] = [war()]): WarsReply => ({ countryNames: [], branchId: 'branch', available: true, viewpoints: [], wars, unobserved: [], capitulationsUnobserved: [], warnings: [] });

test('the whole-branch wars reply war contribution reads is validated whole', () => {
  const data = reply();
  assert.equal(validateWarsReply(data), data);
  for (const bad of [null, {}, { ...data, available: undefined }, { ...data, countryNames: undefined }, { ...data, viewpoints: undefined }, { ...data, warnings: [null] }, { ...data, wars: [data.wars[0], data.wars[0]] },
    { ...data, wars: [{ ...war(), participants: [{ ...participant('A', 'attacker'), side: 'invented' }] }] },
    { ...data, unobserved: [{ firstHour: 14, lastHour: 14 }] }]) {
    assert.equal(isWarsReply(bad), false);
    assert.throws(() => validateWarsReply(bad), /malformed/);
  }
  assert.equal(isWarsReply(reply([])), true);
});

test('an observation is found at its own hour, never carried past the war', () => {
  const item = { ...war('1', 10, 19), observations: [observation(10, 19), { ...observation(20, 60), name: { text: 'Later name', known: true, reason: null, key: null } }] };
  assert.equal(observationAt(item, 19.9)?.name.text, 'Recorded name');
  assert.equal(labelObservationAt(item, 50), item.observations[1]);
});
