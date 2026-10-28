import assert from 'node:assert/strict';
import test from 'node:test';
import type { FactionDetailReply, FactionGoalDefinition } from '../../generated/dashboard';
import { manpowerRows, poolHeading, poolsOffered, shareOf, shownPool } from './poolsData.ts';
import { completedCount, filterGoals, goalRows, termOrder } from '../faction-goals/factionGoalsData.ts';

const reply = (values: Partial<FactionDetailReply>): FactionDetailReply => ({ resources: null, manpower: null, manpowerUnlocked: null,
  researchUnlocked: null, researchBaseRaw: null, researchCountries: null, doctrineFolders: null, goals: [], completed: [], cancelled: [],
  warnings: [], refusals: [], ...values } as unknown as FactionDetailReply);

test('only the pools the recorded state supplies are offered, and a saved pool falls back', () => {
  assert.deepEqual(poolsOffered(reply({})), [], 'unavailable reads offer nothing, not empty pools');
  const all = reply({ resources: [], manpower: [], manpowerUnlocked: true, researchUnlocked: true, researchBaseRaw: '100000',
    doctrineFolders: [{ ordinal: 0, token: 'mod_folder', name: 'A mod doctrine', reason: null }] });
  assert.deepEqual(poolsOffered(all), ['resources', 'manpower', 'research', 'doctrine']);
  assert.deepEqual(poolsOffered(reply({ manpower: [], manpowerUnlocked: false, researchUnlocked: false, researchBaseRaw: '1' })), [], 'locked sections are not offered');
  assert.equal(shownPool(['manpower', 'doctrine'], 'doctrine'), 'doctrine');
  assert.equal(shownPool(['manpower'], 'resources'), 'manpower');
  assert.equal(shownPool([], 'resources'), null);
});

test('a research pool whose members were not read keeps its base and never says none share', () => {
  const unread = reply({ researchUnlocked: true, researchBaseRaw: '100000', researchCountries: null });
  assert.deepEqual(poolsOffered(unread), ['research']);
  assert.equal(poolHeading(unread, 'research'), 'Members not recorded');
  assert.equal(poolHeading(reply({ researchUnlocked: true, researchBaseRaw: '100000', researchCountries: [] }), 'research'), '0 sharing');
});

test('manpower keeps every native occurrence, largest first, with shares of the recorded total', () => {
  const rows = manpowerRows(reply({ manpower: [{ ordinal: 0, tag: 'A', value: 10 }, { ordinal: 1, tag: 'B', value: 30 }, { ordinal: 2, tag: 'A', value: 10 }, { ordinal: 3, tag: null, value: -5 }] }));
  assert.deepEqual(rows.map(row => [row.key, row.value]), [['1', 30], ['0', 10], ['2', 10], ['3', -5]]);
  assert.equal(shareOf(30, 45), '66.7%');
  assert.equal(shareOf(30, null), '—', 'an unrecorded total gives no share');
  assert.equal(shareOf(30, 0), '—');
});

const definition = (name: string | null, category: string | null, categoryName: string | null, reason: string | null = null) =>
  ({ name, token: name ? 'tok' : null, icon: null, category, categoryName, reason, description: null, progress: [], sections: [], bands: [], conditions: [], rewards: [], flags: [] }) as FactionGoalDefinition;

test('goal rows keep held state, list-only history without dates, and terms in native order', () => {
  const data = reply({
    goals: [{ ordinal: 0, definition: definition('First', 'medium_term', 'Medium'), status: 0, progressRaw: '50000', progressPercent: 50, activeSections: [], date: 0 },
      { ordinal: 1, definition: definition(null, 'short_term', null, 'Name unresolved'), status: 9, progressRaw: '0', progressPercent: null, activeSections: [], date: 0 }],
    completed: [definition('Done', 'medium_term', 'Medium')], cancelled: [definition('Dropped', 'long_term', 'Long')],
  });
  const rows = goalRows(data);
  assert.deepEqual(rows.map(row => [row.name, row.state, row.statusText, row.progress, row.history]), [
    ['First', 'inProgress', 'In progress', 0.5, false], ['Goal name unavailable', 'other', 'State 9', null, false],
    ['Done', 'completed', 'Completed', null, true], ['Dropped', 'cancelled', 'Cancelled', null, true]]);
  assert.equal(rows[2].progress, null, 'a completed list entry is not assumed to be at 100%');
  assert.deepEqual([...termOrder(rows)], [['medium_term', 0], ['short_term', 1], ['long_term', 2]]);
  assert.deepEqual(filterGoals(rows, 'inProgress').map(row => row.name), ['First', 'Goal name unavailable']);
  assert.deepEqual(filterGoals(rows, 'completed').map(row => row.name), ['Done']);
  assert.equal(filterGoals(rows, 'all').length, 4);
  assert.equal(completedCount(rows), 1);
});

test('a member’s manpower history keeps known values and breaks at unknown stretches', async () => {
  const { manpowerLine } = await import('./manpowerLine.ts');
  const line = manpowerLine([{ firstHour: 0, lastHour: 9, value: 100, startsSegment: false }, { firstHour: 10, lastHour: 19, value: null, startsSegment: false },
    { firstHour: 20, lastHour: 29, value: 0, startsSegment: false }, { firstHour: 30, lastHour: 39, value: -5, startsSegment: false }]);
  assert.deepEqual(line.map(run => [run.firstHour, run.value, run.breakBefore ?? false]), [[0, 100, true], [20, 0, true], [30, -5, false]]);
  const { extentOf } = await import('../../charts/series.ts');
  assert.deepEqual(extentOf(line, 0, 39), [-5, 100], 'a negative contribution stays inside the fitted value range');
});

test('the manpower drill keeps the holder the tag named when it was picked', async () => {
  const { chosenMembers } = await import('./manpowerLine.ts');
  const member = (key: string, slot: number | null, firstHour: number, lastHour: number, tag = 'REUSED') => ({ key, tag, slot, countryId: slot, runs: [{ firstHour, lastHour, value: 1, startsSegment: false }] });
  // The old holder's contribution is unknown from 100 on, past the hour the tag passed on.
  const old = { ...member('old', 1, 0, 99), runs: [{ firstHour: 0, lastHour: 99, value: 1, startsSegment: false }, { firstHour: 100, lastHour: 300, value: null, startsSegment: false }] };
  const members = [old, member('new', 2, 200, 300), member('unknown', null, 0, 99), member('old again', 1, 400, 500, 'RENAMED')];
  assert.deepEqual(chosenMembers(members, 'REUSED', 50).map(item => item.key), ['old', 'old again'],
    'the holder picked keeps every occurrence of its own, by slot and country id; a later holder of the tag and an unknown holder are not joined');
  assert.deepEqual(chosenMembers(members, 'REUSED', 250).map(item => item.key), ['new'], 'an earlier holder’s unknown tail is not joined to the new holder');
  assert.deepEqual(chosenMembers(members, 'REUSED', 150).map(item => item.key), [], 'no holder is proven at an hour only an unknown tail covers');
});
