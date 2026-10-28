import assert from 'node:assert/strict';
import test from 'node:test';
import type { FactionDetailReply, FactionGoalDefinition, FactionGoalDetail } from '../../generated/dashboard';
import { HOUR_EPOCH } from '../../dashboard/calendar.ts';
import { extracted, fixedWhole, goalGroups, goalProgress, manifestGoal, manifestMarks, rewardLines, rewardRows, yearStarts } from './factionDrillData.ts';

const definition = (token: string, extra: Partial<FactionGoalDefinition> = {}): FactionGoalDefinition => ({ progress: [], sections: [], bands: [],
  conditions: [], rewards: [], flags: [], token, name: token, description: null, icon: null, category: null, categoryName: null, reason: null, ...extra });
const goal = (ordinal: number, token: string, extra: Partial<FactionGoalDetail> = {}, defined: Partial<FactionGoalDefinition> = {}): FactionGoalDetail =>
  ({ ordinal, definition: definition(token, defined), status: 0, progressRaw: '0', activeSections: [], progressPercent: 0, date: 0, ...extra });

test('reward text drops the game\'s colour and country markup and headings with nothing under them', () => {
  assert.deepEqual(rewardLines('Add \u0011H100\u0011! faction power projection in \u0011HAxis\u0011!\n \nTarget State Controllers:\n(Norrbotten, Västerbotten):\n   Add 1 Infrastructure.\n'), [
    { text: 'Add 100 faction power projection in Axis', heading: false },
    { text: 'Target State Controllers', heading: true },
    { text: 'Norrbotten, Västerbotten', heading: true },
    { text: 'Add 1 Infrastructure.', heading: false },
  ]);
  // A scope whose own effects were not described is left out entirely.
  assert.deepEqual(rewardLines('Add 200 power\nEvery Faction Member:\n(\u0012GERGerman Reich, \u0012RKHProtektorat):\n').map(line => line.text), ['Add 200 power']);
  assert.deepEqual(rewardLines('Add 500 power\n \nThe 2 Faction Members with the highest influence:').map(line => line.text), ['Add 500 power']);
  const rows = rewardRows(definition('g', { rewards: [{ kind: 'complete_effect', text: 'The Faction Leader:\nAdd 1 point', reasons: [] }, { kind: 'select_effect', text: 'ignored', reasons: [] }] }), 'Gave');
  assert.deepEqual(rows.map(row => row.label), ['Gave', 'The Faction Leader', 'Add 1 point']);
  assert.deepEqual(rewardRows(definition('g'), 'Gave'), []);
});

test('the manifest is the goal its definition names, with its bar only for a real share and its named thresholds', () => {
  const manifest = goal(0, 'manifest', { progressPercent: 92.105, activeSections: [0] }, { bands: [{ name: 'Manifesto Fulfilled', min: 0.75, max: null }] });
  const reply = { manifest: definition('manifest'), goals: [manifest, goal(1, 'a', { progressPercent: 92233720124.192 }, { category: 'medium_term', categoryName: 'Medium Term' }),
    goal(2, 'b', {}, { category: 'short_term', categoryName: 'Short Term' }), goal(3, 'c', {}, { category: 'modded_term' })] } as unknown as FactionDetailReply;
  assert.equal(manifestGoal(reply), manifest);
  assert.equal(goalProgress(manifest), 92.105);
  assert.equal(goalProgress(reply.goals[1]), null, 'a placeholder outside 0–100% is no progress');
  assert.deepEqual(manifestMarks(manifest), { marks: [{ at: 75, name: 'Manifesto Fulfilled' }], reached: 'Manifesto Fulfilled' });
  assert.deepEqual(manifestMarks({ ...manifest, activeSections: [] }).reached, null);
  assert.deepEqual(goalGroups(reply, manifest).map(group => [group.name, group.goals.map(entry => entry.ordinal)]),
    [['Short Term', [2]], ['Medium Term', [1]], ['modded term', [3]]]);
});

test('resources keep amounts whose names are unavailable and read their fixed-point amounts', () => {
  const resources = [{ ordinal: 3, token: null, name: null, icon: null, raw: '50000000', reason: 'Faction resource token spelling unavailable' },
    { ordinal: 1, token: 'oil', name: 'Oil', icon: null, raw: '17400000', reason: null },
    { ordinal: 2, token: 'steel', name: 'Steel', icon: null, raw: '113100000', reason: null }];
  assert.deepEqual(extracted(resources).map(entry => [entry.resource.ordinal, entry.value]), [[2, 1131], [3, 500], [1, 174]]);
  assert.equal(fixedWhole('230698765'), 2306);
  assert.equal(fixedWhole(null), null);
});

test('the growth chart labels each game year inside its range', () => {
  const year = 365 * 24;
  assert.deepEqual(yearStarts(HOUR_EPOCH + 1936 * year + 5, HOUR_EPOCH + 1938 * year).map(start => start.year), [1937, 1938]);
});
