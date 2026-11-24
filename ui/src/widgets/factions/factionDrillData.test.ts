import assert from 'node:assert/strict';
import test from 'node:test';
import type { FactionDetailReply, FactionGoalDefinition, FactionGoalRewardsReply, FactionHeldGoal } from '../../generated/dashboard';
import { valuesOf } from '../../charts/live.ts';
import { HOUR_EPOCH } from '../../dashboard/calendar.ts';
import { FIXED5_MAX, goalProgressKey, resourceKey } from '../../dashboard/factionDetails.ts';
import { goalGroups, goalPercent, manifestGoal, manifestMarks, resourceOrder, rewardLines, rewardRows, wholeOf, yearStarts } from './factionDrillData.ts';

const definition = (token: string, extra: Partial<FactionGoalDefinition> = {}): FactionGoalDefinition => ({ bands: [], token, name: token, description: null,
  icon: null, category: null, categoryName: null, reason: null, ...extra });
const held = (ordinal: number, index: number, extra: Partial<FactionHeldGoal> = {}): FactionHeldGoal => ({ ordinal, definition: index, status: 0, activeSections: [], ...extra });
const described = (text: string | null, reason: string | null = null): FactionGoalRewardsReply => ({ goal: 'g', text, reason, warnings: [] });

test('reward text drops the game\'s colour and country markup and headings with nothing under them', () => {
  assert.deepEqual(rewardLines('Add \u0011H100\u0011! faction power projection in \u0011HAxis\u0011!\n \nTarget State Controllers:\n(Norrbotten, Västerbotten):\n   Add 1 Infrastructure.\n'), [
    { text: 'Add 100 faction power projection in Axis', heading: false },
    { text: 'Target State Controllers', heading: true },
    { text: 'Norrbotten, Västerbotten', heading: true },
    { text: 'Add 1 Infrastructure.', heading: false },
  ]);
  assert.deepEqual(rewardLines('Add 200 power\nEvery Faction Member:\n(\u0012GERGerman Reich, \u0012RKHProtektorat):\n').map(line => line.text), ['Add 200 power']);
  assert.deepEqual(rewardLines('Add 500 power\n \nThe 2 Faction Members with the highest influence:').map(line => line.text), ['Add 500 power']);
});

test('a goal\'s reward rows say it gives nothing described, is loading, or cannot be described, never an empty heading', () => {
  const rows = rewardRows({ kind: 'described', reply: described('The Faction Leader:\nAdd 1 point') }, 'Gave');
  assert.deepEqual(rows.map(row => row.label), ['Gave', 'The Faction Leader', 'Add 1 point']);
  assert.deepEqual(rewardRows({ kind: 'described', reply: described(null) }, 'Gave'), [], 'no completion effect authored');
  assert.deepEqual(rewardRows({ kind: 'loading' }, 'On completion').map(row => row.label), ['On completion', 'Loading rewards…']);
  assert.deepEqual(rewardRows({ kind: 'described', reply: described(null, 'The faction has no leader at this hour') }, 'Gave').map(row => [row.label, row.tone]),
    [['Gave', undefined], ['Rewards not described: The faction has no leader at this hour', 'muted']]);
  assert.match(rewardRows({ kind: 'failed', reason: 'Faction goal "x" is absent from recorded content' }, 'Gave')[1].label, /^Rewards unavailable: /);
});

test('the manifest is the held goal its definition names, its bar only for a real share, with its named thresholds', () => {
  const reply = { manifest: 0, definitions: [
    definition('manifest', { bands: [{ name: 'Manifesto Fulfilled', min: 0.75, max: null }] }),
    definition('a', { category: 'medium_term', categoryName: 'Medium Term' }),
    definition('b', { category: 'short_term', categoryName: 'Short Term' }),
    definition('c', { category: 'modded_term' }),
  ], goals: [held(0, 0, { activeSections: [0] }), held(1, 1), held(2, 2), held(3, 3)] } as unknown as FactionDetailReply;
  const manifest = manifestGoal(reply)!;
  assert.equal(manifest.goal, reply.goals[0]);
  assert.equal(manifest.definition.token, 'manifest');
  const values = valuesOf(new Map([[goalProgressKey(0), 92_105], [goalProgressKey(1), FIXED5_MAX], [goalProgressKey(2), 150_000]]), 'Not recorded');
  assert.equal(goalPercent(values, manifest.goal), 92.105);
  assert.equal(goalPercent(values, reply.goals[1]), null, 'the game\'s placeholder is no progress');
  assert.equal(goalPercent(values, reply.goals[2]), null, 'outside 0–100% is no share');
  assert.equal(goalPercent(values, reply.goals[3]), null, 'a cell not recorded is no progress');
  assert.deepEqual(manifestMarks(manifest), { marks: [{ at: 75, name: 'Manifesto Fulfilled' }], reached: 'Manifesto Fulfilled' });
  assert.deepEqual(manifestMarks({ ...manifest, goal: { ...manifest.goal, activeSections: [] } }).reached, null);
  assert.deepEqual(goalGroups(reply, manifest).map(group => [group.name, group.goals.map(entry => entry.goal.ordinal)]),
    [['Short Term', [2]], ['Medium Term', [1]], ['modded term', [3]]]);
  assert.equal(manifestGoal({ ...reply, manifest: 3, goals: reply.goals.slice(0, 3) }), null, 'a manifest not held is not shown');
});

test('pooled resources are placed largest first by their live amounts, an amount not recorded last, and whole numbers truncate', () => {
  const resources = [{ ordinal: 3, token: null, name: null, icon: null, reason: 'Faction resource token spelling unavailable' },
    { ordinal: 1, token: 'oil', name: 'Oil', icon: null, reason: null },
    { ordinal: 2, token: 'steel', name: 'Steel', icon: null, reason: null },
    { ordinal: 4, token: 'rubber', name: 'Rubber', icon: null, reason: null }];
  const values = valuesOf(new Map([[resourceKey(3), 500], [resourceKey(1), 174], [resourceKey(2), 1131]]), 'Not recorded');
  assert.deepEqual([...resourceOrder(resources, values)], [[2, 0], [3, 1], [1, 2], [4, 3]]);
  assert.equal(wholeOf(2306.98765), 2306);
  assert.equal(wholeOf(-1.5), -1);
  assert.equal(wholeOf(undefined), null);
});

test('the growth chart labels each game year inside its range', () => {
  const year = 365 * 24;
  assert.deepEqual(yearStarts(HOUR_EPOCH + 1936 * year + 5, HOUR_EPOCH + 1938 * year).map(start => start.year), [1937, 1938]);
});
