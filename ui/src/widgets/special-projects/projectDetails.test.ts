import assert from 'node:assert/strict';
import test from 'node:test';
import { detailsStructure, detailTexts, validateProjectDetails, validateProjectEffects } from './projectDetailsData.ts';
import type { SpecialProjectDetails } from '../../generated/dashboard';

const person = { assigned: true, id: 7, name: 'Scientist', portrait: null, missing: null };
const label = { token: 'mod_project', name: 'Project' };
const reply = {
  hour: 24, unavailable: [], warnings: [],
  breakthroughs: [{ specialization: label, points: -12345, readHour: 24 }],
  scientists: [{ character: 7, person, active: 253, injuryDays: -1, injuryHour: 24, visible: true, visibilityHour: 24, stateRefusals: {},
    skills: [{ specialization: label, level: -3, experienceRaw: '9223372036854775807' }], traits: [label, label] }],
  programs: [{ id: 3, province: 9, readHour: 24,
    facility: { speedRaw: '87654', unavailable: null, sources: [{ kind: 'Scientist trait', unit: 'factor', label, modifier: label, assignment: 1, scientistReference: '30064771145', valueRaw: '123456' }] },
    assignments: [{ ordinal: 0, reference: '30064771145', person, target: 5, current: -2, timed: 1 }, { ordinal: 1, reference: '30064771145', person, target: 5, current: 2, timed: 0 }] }],
  projects: [{ owner: 5, id: 1, label, readHour: 24, pendingKnown: true, pending: { label, description: null, options: [{ label }], unavailable: null },
    choices: [{ ordinal: 0, date: 23, reward: label, option: label, province: -1, progress: 12345, characterReference: '18446744073709551615', country: 5, scientist: 7, state: null }],
    seed: 4294967295, variables: [{ name: 'mod_array^0', valueRaw: '-9223372036854775808' }] }],
} satisfies SpecialProjectDetails;

test('project details keep duplicate traits, supporting assignments, pending choices and exact variables, and refuse what cannot be', () => {
  assert.equal(validateProjectDetails(reply), reply);
  assert.throws(() => validateProjectDetails({ ...reply, programs: [{ ...reply.programs[0], facility: { ...reply.programs[0].facility, speedRaw: '9223372036854775808' } }] }), /Invalid/);
  assert.throws(() => validateProjectDetails({ ...reply, projects: [{ ...reply.projects[0], choices: [{ ...reply.projects[0].choices[0], characterReference: '18446744073709551616' }] }] }), /Invalid/);
  assert.throws(() => validateProjectDetails({ ...reply, breakthroughs: [{ ...reply.breakthroughs[0], readHour: 25 }] }), /Invalid/, 'a read after the hour shown');
  const unread = { ...reply, scientists: null, programs: null, projects: null, breakthroughs: null };
  assert.equal(validateProjectDetails(unread), unread);
  for (const roster of ['scientists', 'programs', 'projects', 'breakthroughs']) assert.throws(() => validateProjectDetails({ ...reply, [roster]: undefined }), /Invalid/);
  assert.throws(() => validateProjectDetails({ ...reply, scientists: [{ ...reply.scientists[0], visibilityHour: 25 }] }), /Invalid/);
  assert.throws(() => validateProjectDetails({ ...reply, programs: [{ ...reply.programs[0], readHour: 25 }] }), /Invalid/);
  assert.throws(() => validateProjectDetails({ ...reply, projects: [{ ...reply.projects[0], pendingKnown: false }] }), /Invalid/);
  assert.throws(() => validateProjectDetails({ ...reply, warnings: undefined }), /Invalid/);
  const effects = { hour: 24, output: { text: 'Output', refusals: { 'Scope unavailable': 1 } }, options: [{ text: '', refusals: {} }],
    iterations: [{ ordinal: 0, effects: { text: 'Gained a bonus', refusals: {} } }], unavailable: null, warnings: [] };
  assert.equal(validateProjectEffects(effects), effects);
  assert.throws(() => validateProjectEffects({ ...effects, output: { text: 'Output', refusals: { reason: 0 } } }), /Invalid/);
  assert.throws(() => validateProjectEffects({ ...effects, iterations: [{ ordinal: -1, effects: effects.iterations[0].effects }] }), /Invalid/);
  assert.throws(() => validateProjectEffects({ ...effects, iterations: undefined }), /Invalid/);
});

test('a daily read moves only painted texts: its read date, counts and values never change the structure that renders', () => {
  const next = { ...reply, hour: 48,
    scientists: [{ ...reply.scientists[0], active: 0, injuryDays: 6, injuryHour: 48, visibilityHour: 48, skills: [{ ...reply.scientists[0].skills[0], level: -2, experienceRaw: '100000' }] }],
    breakthroughs: [{ ...reply.breakthroughs[0], points: 20000, readHour: 48 }],
    programs: [{ ...reply.programs[0], readHour: 48, facility: { ...reply.programs[0].facility, speedRaw: '100000', sources: [{ ...reply.programs[0].facility.sources[0], valueRaw: '1' }] },
      assignments: [{ ...reply.programs[0].assignments[0], current: 3 }, reply.programs[0].assignments[1]] }],
    projects: [{ ...reply.projects[0], readHour: 48, variables: [{ name: 'mod_array^0', valueRaw: '5' }] }] } satisfies SpecialProjectDetails;
  assert.equal(detailsStructure(next), detailsStructure(reply));
  const reseeded = { ...next, projects: [{ ...next.projects[0], seed: 123 }] };
  assert.equal(detailsStructure(reseeded), detailsStructure(next));
  assert.equal(detailTexts(reseeded).get('j:5:1:seed'), '123');
  assert.equal(detailsStructure({ ...Object.fromEntries(Object.entries(reply).reverse()) } as typeof reply), detailsStructure(reply));
  const before = detailTexts(reply), after = detailTexts(next);
  assert.deepEqual([...after.keys()], [...before.keys()], 'the same elements are painted');
  const changed = [...after].filter(([key, text]) => before.get(key) !== text).map(([key]) => key);
  assert.deepEqual(changed, ['s:7:state', 's:7:injury', 's:7:visibility', 's:7:skill:0', 'b:0', 'p:3:read', 'p:3:speed', 'p:3:source:0', 'p:3:assignment:0', 'j:5:1:read', 'j:5:1:var:0']);
  assert.equal(after.get('s:7:injury'), `6 injury recovery days · read ${after.get('j:5:1:read')!.slice(' · read '.length)}`);
  assert.equal(after.get('p:3:assignment:0'), ' · Delayed unassignment: 2 days remaining (3 / 5)');
  assert.equal(after.get('p:3:assignment:1'), '', 'an untimed assignment shows no countdown');
  assert.notEqual(detailsStructure({ ...next, scientists: [] }), detailsStructure(next));
  assert.notEqual(detailsStructure({ ...next, projects: [{ ...next.projects[0], pending: null }] }), detailsStructure(next));
  assert.notEqual(detailsStructure({ ...next, programs: [{ ...next.programs[0], assignments: [{ ...next.programs[0].assignments[0], person: { ...person, name: 'Other' } }, next.programs[0].assignments[1]] }] }), detailsStructure(next));
  const unread = { ...next, scientists: [{ ...next.scientists[0], injuryDays: null, injuryHour: null, visible: null, visibilityHour: null, active: null }] };
  assert.equal(detailTexts(unread).get('s:7:injury'), 'Injury recovery unavailable.', 'a value not read says so, never zero');
  assert.equal(detailTexts(unread).get('s:7:visibility'), 'Roster visibility unavailable');
  assert.equal(detailTexts(unread).get('s:7:state'), 'State unavailable');
});
