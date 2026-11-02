import assert from 'node:assert/strict';
import test from 'node:test';
import { memberNameHour, upgradeLanes } from './upgradesData.ts';
import type { FactionRulesReply as RulesReply, FactionSelectionRun as SelectionRun } from '../../generated/dashboard';

const run = (firstHour: number, lastHour: number, token: string | null, name: string | null = token): SelectionRun =>
  ({ firstHour, lastHour, token, name, reason: name ? null : 'not in content', startsSegment: false });
const reply: RulesReply = {
  rules: [{ slot: 0, runs: [run(0, 49, 'mod_rule'), run(50, 99, 'other_rule', null)] }, { slot: 1, runs: [run(0, 99, null)] }],
  programmes: [{ slot: 0, runs: [run(0, 20, 'programme')] }],
  minimums: [{ group: 3, runs: [run(10, 99, 'minimum')] }],
  members: [{ key: 'a', tag: 'mod:Ω', slot: 1, countryId: 1, group: 3, runs: [run(30, 99, 'upgrade')] },
    // As the backend closes a lane that went away: a run without a selection from then on,
    // reaching past the hour another country took the tag.
    { key: 'b', tag: 'mod:Ω', slot: 2, countryId: 9, group: 3, runs: [run(10, 20, 'earlier holder'), run(21, 99, null)] }],
  doctrines: [], groups: [{ group: 3, name: 'A mod upgrade group' }], unobserved: [], pending: [], warnings: [],
} as unknown as RulesReply;
const names = { groupName: (group: number) => group === 3 ? 'A mod upgrade group' : 'unnamed', country: (tag: string, hour: number) => ({ name: `Country ${tag} at ${hour}` }) };

test('lanes sit under section headings, unrecorded selections draw nothing and Live cuts at the playhead', () => {
  const { lanes, info } = upgradeLanes(reply, 60, null, { endedProgrammes: true, ...names });
  assert.deepEqual(lanes.map(lane => lane.label), ['Rules', 'Rule 1', 'Shared programmes', 'Programme 1', 'Faction minimum upgrades', 'A mod upgrade group',
    'Member upgrades', 'Country mod:Ω at 99 · A mod upgrade group', 'Country mod:Ω at 20 · A mod upgrade group'], 'a slot with no recorded selection is left out');
  assert.equal(memberNameHour({ runs: [run(10, 20, 'earlier holder'), run(21, 99, null)] }), 20, 'the closing run without a selection never names the lane');
  assert.equal(memberNameHour({ runs: [run(0, 99, null)] }), null);
  assert.deepEqual(lanes.filter(lane => lane.id.startsWith('member:')).map(lane => lane.id), ['member:a', 'member:b'], 'two holders of one tag and group keep their own lanes');
  const rule = lanes.find(lane => lane.label === 'Rule 1')!;
  assert.deepEqual(rule.spans.map(span => [span.label, span.emphasis]), [['mod_rule', false], ['Name unavailable', true]]);
  assert.equal(lanes.find(lane => lane.label === 'Programme 1')!.muted, true, 'an ended programme is muted');
  assert.equal(info.get('rule:0')!.section, 'Rules');
  const live = upgradeLanes(reply, 40, 40, { endedProgrammes: true, ...names });
  assert.deepEqual(live.lanes.find(lane => lane.label === 'Rule 1')!.spans.map(span => [span.lastHour, span.openEnd]), [[40, true]], 'later selections are not shown yet');
  assert.equal(upgradeLanes(reply, 60, null, { endedProgrammes: false, ...names }).lanes.some(lane => lane.label === 'Programme 1'), false);
});

test('faction member lane info retains the country failure used in its tooltip subtitle', () => {
  const failure = new Error('Synthetic faction member name decoder failure');
  const { info } = upgradeLanes(reply, 60, null, { endedProgrammes: true, ...names,
    country: () => ({ name: 'Couldn’t load country name.', nameFailure: failure }) });
  assert.equal(info.get('member:a')?.nameFailure, failure);
  assert.match(info.get('member:a')!.slot, /Couldn’t load country name/);
});
