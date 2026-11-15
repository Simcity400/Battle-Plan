import test from 'node:test';
import assert from 'node:assert/strict';
import { FIXED5_MAX, goalProgressKey, validateFactionDetails } from '../../dashboard/factionDetails.ts';
import { valuesOf } from '../../charts/live.ts';
import { filterGoals, goalDiagnostics, goalRows, goalTableValues, progressText, rowProgress, termOrder } from './factionGoalsData.ts';
import type { FactionDetailReply, FactionGoalDefinition } from '../../generated/dashboard';

const definition = (token: string, extra: Partial<FactionGoalDefinition> = {}): FactionGoalDefinition => ({ token, name: `Mod ${token}`, description: null, icon: null,
  category: 'mod_term', categoryName: 'A mod term', bands: [], reason: null, ...extra });
const ARTWORK = 'Faction goal artwork GFX_mod; group: sprite is absent';

/** A modded faction at one hour: two goals held (one cancelling), one completed, one cancelled whose name is unknown. */
const reply = (): FactionDetailReply => validateFactionDetails({
  branchId: '1', faction: '9', logo: null,
  definitions: [definition('mod_a'), definition('mod_b', { reason: ARTWORK, category: 'other_term', categoryName: null }),
    definition('mod_c', { name: null, reason: 'Faction goal "mod_c" is absent from recorded content' })],
  manifest: null, goals: [{ ordinal: 0, definition: 0, status: 0, activeSections: [] }, { ordinal: 1, definition: 1, status: 4, activeSections: [] }],
  completed: [0], cancelled: [2], goalReason: null, resources: null, manpower: null, manpowerUnlocked: null, researchUnlocked: null,
  researchCountries: null, doctrineFolders: null, refusals: [{ fact: 'goal artwork', count: 1, reason: ARTWORK }], warnings: ['mod content warning'],
});

test('the hour\'s goals are rows by their lists, progress read from each held goal\'s live cell', () => {
  const rows = goalRows(reply());
  assert.deepEqual(rows.map(row => [row.key, row.name, row.statusText, row.state, row.history]), [
    ['goal:0', 'Mod mod_a', 'In progress', 'inProgress', false], ['goal:1', 'Mod mod_b', 'Cancelling', 'inProgress', false],
    ['completed:0', 'Mod mod_a', 'Completed', 'completed', true], ['cancelled:0', 'Goal name unavailable', 'Cancelled', 'cancelled', true]]);
  const values = valuesOf(new Map([[goalProgressKey(0), 25_000], [goalProgressKey(1), FIXED5_MAX]]), 'Not read yet');
  assert.equal(progressText(rowProgress(values, rows[0]), 1), '25.0%');
  // Only the game's saturated cache marks a goal without a progress measure; a history entry holds no progress.
  assert.equal(progressText(rowProgress(values, rows[1]), 1), 'No progress measure');
  assert.equal(progressText(rowProgress(values, rows[2]), 1), 'Not recorded');
  // The table's painted column: a share of the goal, the unmeasured one missing for its reason, never a full bar.
  const table = goalTableValues(values);
  assert.equal(table.get(goalProgressKey(0)), 0.25);
  assert.equal(table.get(goalProgressKey(1)), undefined);
  assert.equal(table.missing(goalProgressKey(1)), 'No progress measure');
  assert.equal(table.missing(goalProgressKey(5)), 'Not read yet');
  assert.deepEqual(filterGoals(rows, 'completed').map(row => row.key), ['completed:0']);
  assert.deepEqual(filterGoals(rows, 'inProgress').map(row => row.key), ['goal:0', 'goal:1']);
  assert.deepEqual([...termOrder(rows)], [['mod_term', 0], ['other_term', 1]]);
});

test('what cannot be shown is said: reply warnings, artwork refusals, unnamed goals and terms', () => {
  const data = reply();
  assert.deepEqual(goalDiagnostics(data, goalRows(data)), ['mod content warning', `goal artwork (1): ${ARTWORK}`,
    '2 goal names are unavailable.', '1 goal term has no name in the recorded content.']);
  assert.throws(() => validateFactionDetails({ ...data, goals: [{ ordinal: 0, definition: 7, status: 0, activeSections: [] }] }), /malformed/);
});

test('a saved faction outside its own life is absent at that hour, never a failed read', async () => {
  // Run 14 at 31 May 1938: the African Union, founded in the 1940s, showed "Couldn't load" with the product's refusal.
  const { createServer } = await import('vite');
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { chosenFaction, noFactionMessage } = await vite.ssrLoadModule('/ui/src/dashboard/factionChoice.ts') as typeof import('../../dashboard/factionChoice');
    const union = { id: '30', name: 'African Union', nameReason: null, recordedName: '', firstHour: 500, lastHour: 900, names: [], members: [], leaders: [], incomplete: [] };
    const groups = { factions: [union], unobserved: { factions: [], humans: [], wars: [] } } as unknown as import('../../generated/dashboard').CountryGroupReply;
    for (const hour of [100, 901]) assert.deepEqual(chosenFaction(groups, '30', 'GER', hour), { faction: null, reason: 'absent' });
    assert.equal(chosenFaction(groups, '30', 'GER', 500).faction, union);
    assert.equal(noFactionMessage('absent', true), 'This faction did not exist at this time.');
  } finally { await vite.close(); }
});
