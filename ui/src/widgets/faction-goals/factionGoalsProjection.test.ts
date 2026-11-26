import assert from 'node:assert/strict';
import { test } from 'node:test';
import { liveKey } from '../../charts/live.ts';
import { valuesOf } from '../../test-support/liveValues.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { knownValue, notApplicable, notRecorded } from '../../families/value.ts';
import type { CountryFactionGoalsReply, CountryGoal, FactionGoalDefinition } from '../../generated/dashboard';
import { factionGoalsDeclaration, factionGoalsProjection } from './factionGoalsProjection.ts';
import type { FamilyPlanContext } from '../../families/plan.ts';

const definition = (name: string | null): FactionGoalDefinition => ({
  token: name, name, description: null, icon: null, category: null, categoryName: null, bands: [], reason: name ? null : 'Goal name is not localised',
} as FactionGoalDefinition);

const goal = (key: string, definition: number, state: CountryGoal['state'], ordinal: number | null, unrecognisedStatus: number | null = null): CountryGoal =>
  ({ key, definition, state, ordinal, unrecognisedStatus });

const reply = (extra: Partial<CountryFactionGoalsReply> = {}): CountryFactionGoalsReply => ({
  faction: '9', definitions: [definition('Conquest and Domination'), definition('Secure North Africa'), definition(null)],
  goals: [goal('goal:0', 0, 'inProgress', 0), goal('goal:1', 1, 'inProgress', 1), goal('completed:0', 2, 'completed', null)],
  goalReason: null, membershipMissing: null, warnings: [], ...extra,
});

const project = (raw: unknown, live: Map<string, number> = new Map()) =>
  factionGoalsProjection.project(createBoundedReadView({ hour: 50, panels: new Map([['main', raw]]), live: new Map([['main', valuesOf(live, 'Not in the answer')]]) }), {});

test('a held goal shows its recorded progress; one without a measure does not apply; a completed goal holds none; a missing cell is missing', () => {
  const model = project(reply(), new Map([[liveKey('goal:0', 'progress'), 96100], [liveKey('goal:1', 'progress'), 92233720124192]]));
  const [held, unmeasured, completed] = model.rows;
  assert.deepEqual(held!.cells.progress!.value, knownValue(96.1));
  assert.deepEqual(held!.cells.state!.value, knownValue('In progress'));
  assert.equal(unmeasured!.cells.progress!.value.kind, 'notApplicable');
  assert.deepEqual(completed!.cells.progress!.value, notApplicable('A completed goal holds no progress'));
  assert.deepEqual(completed!.cells.goal!.value, notRecorded('Goal name is not localised'));
  assert.equal(held!.muted, undefined);
  assert.equal(completed!.muted, true, 'completed goals dim beneath active goals');
  assert.equal(held!.cells.icon!.art?.status === 'unavailable' && held!.cells.icon!.art.reason.includes('Conquest and Domination'), true, 'a missing goal icon names the goal');
  assert.equal(held!.cells.icon!.artName, 'Conquest and Domination');
  assert.equal(project(reply()).rows[0]!.cells.progress!.value.kind, 'notRecorded', 'no progress cell is never zero');
});

test('the goals filter goes to the product; an unrecognised state is not called in progress; membership unknown, no faction and unrecorded goals are the body states', () => {
  const params = (goals: string | undefined) => {
    const ctx = { track: { source: { id: 's', revision: 'r' }, axisOffset: 0 }, branch: { id: '1' }, countryTag: 'GER', options: goals === undefined ? {} : { goals } } as unknown as FamilyPlanContext;
    return (factionGoalsDeclaration.sources(ctx, { following: true, staticSpan: null }).entries[0]!.read as unknown as { params: { goals: string } }).params.goals;
  };
  assert.deepEqual([params('completed'), params('inProgress'), params(undefined)], ['completed', 'inProgress', 'all']);
  const strange = project(reply({ goals: [goal('goal:5', 0, 'unrecognised', 5, 9)] })).rows[0]!;
  assert.equal(strange.cells.state!.value.kind, 'notRecorded');
  assert.equal(strange.cells.state!.tone, 'grey');
  const cancelled = project(reply({
    definitions: [{ token: 'goal_alpha', name: null, description: null, icon: null, category: null, categoryName: null, bands: [], reason: 'Goal name is not localised' } as FactionGoalDefinition],
    goals: [goal('cancelled:0', 0, 'cancelled', null)],
  })).rows[0]!;
  assert.equal(cancelled.muted, true);
  assert.equal(cancelled.cells.icon!.artName, 'goal_alpha');
  assert.equal(cancelled.cells.icon!.art?.status === 'unavailable' && cancelled.cells.icon!.art.reason === 'goal_alpha: Goal name is not localised', true);
  const unknown = project(reply({ faction: null, membershipMissing: 'Faction memberships are not known at this hour' }));
  assert.deepEqual([unknown.emptyKind, unknown.empty], ['unavailable', 'Faction memberships are not known at this hour']);
  assert.equal(project(reply({ faction: null })).emptyKind, 'empty');
  assert.equal(project(reply({ goals: [], definitions: [], goalReason: 'Goals were not read at this hour' })).emptyKind, 'unavailable');
});
