import test from 'node:test';
import assert from 'node:assert/strict';
import { eventHours, eventsThrough, upgradeLanes } from './upgradesData.ts';
import type { FactionRulesReply, FactionSelectionRun } from '../../generated/dashboard';

const run = (firstHour: number, lastHour: number, name: string | null, startsSegment = false): FactionSelectionRun =>
  ({ firstHour, lastHour, token: name ? `mod_${name}` : 'mod_unnamed', name, reason: name ? null : 'Faction rule name requires localisation', startsSegment });
const reply: FactionRulesReply = {
  rules: [{ slot: 0, runs: [run(0, 49, 'open', true), run(50, 99, 'closed')] }],
  programmes: [{ slot: 0, runs: [run(0, 9, 'old', true)] }],
  minimums: [{ group: 7, runs: [run(20, 99, null, true)] }],
  members: [{ key: '3:5:5:7', tag: 'AAA', slot: 5, countryId: 5, group: 7, runs: [run(60, 99, 'up', true)] }],
  doctrines: [], groups: [{ group: 7, name: 'Mod group', reason: null }], unobserved: [], warnings: [],
};
const options = { endedProgrammes: true, groupName: (group: number) => group === 7 ? 'Mod group' : 'Upgrade group name unavailable',
  country: (tag: string | null) => ({ name: tag ?? 'Country unavailable' }) };

test('the lanes move only where a selection starts or ends: a key of the head counts the events it has passed', () => {
  const events = eventHours(reply);
  assert.deepEqual(events, [0, 10, 20, 50, 60, 100]);
  assert.equal(eventsThrough(events, 15), eventsThrough(events, 19), 'inside a run the key holds');
  assert.notEqual(eventsThrough(events, 19), eventsThrough(events, 20));
});

test('following, only what has begun by the hour is drawn, the held run emphasised and ended lanes muted', () => {
  const { lanes, info } = upgradeLanes(reply, 30, true, options);
  assert.deepEqual(lanes.map(lane => [lane.id, lane.label, lane.muted ?? false]), [
    ['heading:rules', 'Rules', false], ['rule:0', 'Rule 1', false], ['heading:programmes', 'Shared programmes', false], ['programme:0', 'Programme 1', true],
    ['heading:minimums', 'Faction minimum upgrades', false], ['minimum:7', 'Mod group', false]]);
  assert.deepEqual(lanes[1].spans.map(span => [span.firstHour, span.emphasis, span.openEnd]), [[0, true, true]]);
  assert.equal(info.get('minimum:7')?.runs[0].reason, 'Faction rule name requires localisation');
  const whole = upgradeLanes(reply, 99, false, { ...options, endedProgrammes: false });
  assert.deepEqual(whole.lanes.filter(lane => !lane.heading).map(lane => lane.id), ['rule:0', 'minimum:7', 'member:3:5:5:7']);
  assert.equal(whole.lanes.find(lane => lane.id === 'member:3:5:5:7')?.label, 'AAA · Mod group');
});
