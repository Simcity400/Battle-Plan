import test from 'node:test';
import assert from 'node:assert/strict';
import type { FactionDetailReply, FactionGoalDefinition, FactionGoalsReply } from '../../generated/dashboard';
import { asksDetails, filterGoals, goalRows, goalsAt } from './factionGoalsData.ts';
import { currentParts, partAt, splitPart, WHOLE } from './goalParts.ts';

const definition = (token: string, extra: Partial<FactionGoalDefinition> = {}): FactionGoalDefinition => ({ progress: [], sections: [], conditions: [], rewards: [],
  flags: [], token, name: `Mod ${token}`, description: null, icon: null, category: 'mod_term', categoryName: 'A mod term', reason: null, ...extra });
const ARTWORK = 'Faction goal artwork GFX_mod; group: sprite is absent';

/** A modded faction: two goals held, one completing at hour 120; the lists unreadable at 130; absent from 140. */
const history: FactionGoalsReply = {
  runs: [{ firstHour: 100, lastHour: 119, list: 0, startsSegment: false }, { firstHour: 120, lastHour: 129, list: 1, startsSegment: false },
    { firstHour: 130, lastHour: 134, list: 2, startsSegment: false }, { firstHour: 140, lastHour: 150, list: 3, startsSegment: false }],
  lists: [
    { absent: false, reason: null, goals: [{ ordinal: 0, definition: 0, status: 0 }, { ordinal: 1, definition: 1, status: 4 }], completed: [], cancelled: [] },
    { absent: false, reason: null, goals: [{ ordinal: 0, definition: 1, status: 4 }], completed: [0], cancelled: [2] },
    { absent: false, reason: 'Faction goal ordinals are not contiguous', goals: [], completed: [], cancelled: [] },
    { absent: true, reason: null, goals: [], completed: [], cancelled: [] },
  ],
  definitions: [definition('mod_a'), definition('mod_b', { reason: ARTWORK }), definition('mod_c', { name: null, reason: 'Faction goal "mod_c" is absent from recorded content' })],
  progress: [
    { ordinal: 0, runs: [{ firstHour: 100, lastHour: 109, progressRaw: '25000', progressPercent: 25, startsSegment: false },
      { firstHour: 110, lastHour: 119, progressRaw: '50000', progressPercent: 50, startsSegment: false }, { firstHour: 120, lastHour: 129, progressRaw: '0', progressPercent: 0, startsSegment: false }] },
    // A non-finite cache is not progress: its run has no percentage.
    { ordinal: 1, runs: [{ firstHour: 100, lastHour: 119, progressRaw: '-9223372036854775808', progressPercent: null, startsSegment: false }] },
  ],
  unobserved: [], pending: [{ firstHour: 151, lastHour: 160 }],
  coverage: { schema: 34, firstHour: 100, lastHour: 150, complete: false }, channels: [], warnings: ['mod content warning'],
};

test('each playback hour reads the goal history locally, as that hour\'s faction details would list it', () => {
  const at = (hour: number) => { const found = goalsAt(history, hour); assert.equal(found.kind, 'held'); return found.kind === 'held' ? found.view : null!; };
  const early = goalRows(at(105));
  assert.deepEqual(early.map(row => [row.key, row.name, row.statusText, row.progress, row.progressRaw]),
    [['goal:0', 'Mod mod_a', 'In progress', 0.25, '25000'], ['goal:1', 'Mod mod_b', 'Cancelling', null, '-9223372036854775808']]);
  assert.equal(goalRows(at(115))[0].progress, 0.5);
  const later = goalRows(at(125));
  assert.deepEqual(later.map(row => [row.key, row.name, row.state, row.history]),
    [['goal:0', 'Mod mod_b', 'inProgress', false], ['completed:0', 'Mod mod_a', 'completed', true], ['cancelled:0', 'Goal name unavailable', 'cancelled', true]]);
  assert.deepEqual(filterGoals(later, 'completed').map(row => row.key), ['completed:0']);
  // Goal-text refusals are counted per hour, artwork refusals named as faction details name them.
  assert.deepEqual(at(125).warnings, ['1 faction goal texts: Faction goal "mod_c" is absent from recorded content', `1 faction goal texts: ${ARTWORK}`, 'mod content warning']);
  assert.deepEqual(at(125).refusals, [{ fact: 'goal artwork', count: 1, reason: ARTWORK }]);
  const unreadable = at(132);
  assert.equal(unreadable.goalReason, 'Faction goal ordinals are not contiguous');
  // As faction details order them, the unreadable lists come first.
  assert.equal(unreadable.warnings[0], 'Faction goals: Faction goal ordinals are not contiguous');
});

test('an hour the history has not read is not an empty list: it, an unprepared hour and a retried absence ask their own details', () => {
  assert.deepEqual(goalsAt(history, 137), { kind: 'unrecorded' });
  assert.deepEqual(goalsAt(history, 99), { kind: 'unrecorded' }, 'before the first run');
  assert.deepEqual(goalsAt(history, 145), { kind: 'absent' });
  assert.deepEqual(goalsAt(history, 155), { kind: 'pending' });
  for (const hour of [99, 137, 155]) assert.equal(asksDetails(goalsAt(history, hour), false, false), true, `hour ${hour}`);
  assert.equal(asksDetails(goalsAt(history, 145), false, false), false, 'a proven absence is answered locally');
  assert.equal(asksDetails(goalsAt(history, 145), false, true), true, 'retrying it asks only that hour');
  assert.equal(asksDetails(goalsAt(history, 105), false, false), false);
  assert.equal(asksDetails(null, true, false), true, 'a failed history range asks each hour');
  // A list naming a definition the reply lacks is not read.
  const broken = { ...history, definitions: history.definitions.slice(0, 1) };
  assert.deepEqual(goalsAt(broken, 105), { kind: 'unrecorded' });
});

test('an over-budget history range is asked again in halves, and a faction switch never shows the last faction', () => {
  const branch = { firstHour: 100, lastHour: 160 };
  const OVER = 'This product reply exceeds the transport reply budget of 25165824 bytes';
  const halves = splitPart(WHOLE[0], OVER, branch)!;
  assert.deepEqual(halves, [{ firstHour: null, lastHour: 130 }, { firstHour: 131, lastHour: null }]);
  assert.deepEqual(splitPart(halves[1], OVER, branch), [{ firstHour: 131, lastHour: 145 }, { firstHour: 146, lastHour: null }]);
  assert.equal(splitPart({ firstHour: 120, lastHour: 120 }, OVER, branch), null, 'one hour cannot split');
  assert.equal(splitPart(WHOLE[0], 'Background preparation is off.', branch), null, 'other failures are not split');
  assert.equal(partAt(halves, 130), halves[0]);
  assert.equal(partAt(halves, 131), halves[1]);
  assert.equal(partAt(halves, 10_000), halves[1], 'the open end holds a growing branch');
  const client = {} as never;
  const held = { client, identity: 'faction 9', attempt: 0, parts: [{ ...halves[0], data: history }] };
  assert.equal(currentParts(held, client, 'faction 9', 0), held.parts);
  assert.equal(currentParts(held, client, 'faction 10', 0), WHOLE, 'another faction starts empty');
  assert.equal(currentParts(held, {} as never, 'faction 9', 0), WHOLE, 'another client starts empty');
  assert.equal(currentParts(held, client, 'faction 9', 1), WHOLE, 'a retry starts again');
});

test('a faction-details reply still reads as the same rows', () => {
  const reply = { goals: [{ ordinal: 0, definition: definition('mod_a'), status: 1, progressRaw: '100000', activeSections: [], progressPercent: 100, date: 0 }],
    completed: [definition('mod_b')], cancelled: [], goalReason: null, warnings: [], refusals: [] } as unknown as FactionDetailReply;
  assert.deepEqual(goalRows(reply).map(row => [row.key, row.statusText, row.progress]), [['goal:0', 'Completed', 1], ['completed:0', 'Completed', null]]);
});
