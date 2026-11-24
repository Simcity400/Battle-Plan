import assert from 'node:assert/strict';
import { test } from 'node:test';
import { liveKey, valuesOf } from '../../charts/live.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { knownValue, notApplicable, notRecorded } from '../../families/value.ts';
import type { CountryFactionGoalsReply, FactionDetailReply, FactionGoalDefinition } from '../../generated/dashboard';
import { factionGoalsProjection } from './factionGoalsProjection.ts';

const definition = (name: string | null): FactionGoalDefinition => ({
  token: name, name, description: null, icon: null, category: null, categoryName: null, bands: [], reason: name ? null : 'Goal name is not localised',
} as FactionGoalDefinition);

const detail = (extra: Partial<FactionDetailReply> = {}): FactionDetailReply => ({
  branchId: '1', faction: '9', logo: null, definitions: [definition('Conquest and Domination'), definition('Secure North Africa'), definition(null)],
  manifest: null, goals: [{ ordinal: 0, definition: 0, status: 0, activeSections: [] }, { ordinal: 1, definition: 1, status: 0, activeSections: [] }],
  completed: [2], cancelled: [], goalReason: null, resources: null, manpower: null, manpowerUnlocked: null, researchUnlocked: null,
  researchCountries: null, doctrineFolders: null, refusals: [], warnings: [], ...extra,
});

const reply = (extra: Partial<CountryFactionGoalsReply> = {}): CountryFactionGoalsReply => ({ faction: detail(), membershipMissing: null, warnings: [], ...extra });

const project = (raw: unknown, live: Map<string, number> = new Map(), goals: 'all' | 'inProgress' | 'completed' = 'all') =>
  factionGoalsProjection.project(createBoundedReadView({ hour: 50, panels: new Map([['main', raw]]), live: new Map([['main', valuesOf(live, 'Not in the answer')]]) }), { goals });

test('a held goal shows its recorded progress; one without a measure does not apply; a completed goal holds none; a missing cell is missing', () => {
  const model = project(reply(), new Map([[liveKey('goal:0', 'progress'), 96100], [liveKey('goal:1', 'progress'), 92233720124192]]));
  const [held, unmeasured, completed] = model.rows;
  assert.deepEqual(held!.cells.progress!.value, knownValue(96.1));
  assert.deepEqual(held!.cells.state!.value, knownValue('In progress'));
  assert.equal(unmeasured!.cells.progress!.value.kind, 'notApplicable');
  assert.deepEqual(completed!.cells.progress!.value, notApplicable('A completed goal holds no progress'));
  assert.deepEqual(completed!.cells.goal!.value, notRecorded('Goal name is not localised'));
  assert.equal(project(reply()).rows[0]!.cells.progress!.value.kind, 'notRecorded', 'no progress cell is never zero');
});

test('the goals filter keeps the chosen state; membership unknown, no faction and unrecorded goals are the body states', () => {
  assert.deepEqual(project(reply(), new Map(), 'completed').rows.map(row => row.key), ['completed:0']);
  const unknown = project(reply({ faction: null, membershipMissing: 'Faction memberships are not known at this hour' }));
  assert.deepEqual([unknown.emptyKind, unknown.empty], ['unavailable', 'Faction memberships are not known at this hour']);
  assert.equal(project(reply({ faction: null })).emptyKind, 'empty');
  assert.equal(project(reply({ faction: detail({ goals: [], completed: [], goalReason: 'Goals were not read at this hour' }) })).emptyKind, 'unavailable');
});
