import assert from 'node:assert/strict';
import test from 'node:test';
import { REWARDS_PANEL, detailsStructure, detailTexts, effectsRead, optionEffects, rewardsStructure, validateProjectDetails, validateProjectEffects, validateProjectIteration } from './projectDetailsData.ts';
import { panelViews, readingStructure } from '../../draw/panels/views.ts';
import type { ProjectRuntimeDetails, SpecialProjectDetails, SpecialProjectEffects } from '../../generated/dashboard';

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
  projects: [{ owner: 5, id: 1, label, readHour: 24, pendingKnown: true, pending: { label, ordinal: 0, description: null, options: [{ label }], unavailable: null },
    choices: [{ ordinal: 0, date: 23, earlier: false, reward: label, option: label, province: -1, progress: 12345, characterReference: '18446744073709551615', country: 5, scientist: 7, state: null }],
    seed: 4294967295, variables: [{ name: 'mod_array^0', valueRaw: '-9223372036854775808' }], historyReadHour: 23 }],
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
  assert.throws(() => validateProjectDetails({ ...reply, projects: [{ ...reply.projects[0], historyReadHour: 25 }] }), /Invalid/, 'choices read after the hour shown');
  assert.throws(() => validateProjectDetails({ ...reply, projects: [{ ...reply.projects[0], historyReadHour: null }] }), /Invalid/, 'choices with no history read');
  assert.throws(() => validateProjectDetails({ ...reply, projects: [{ ...reply.projects[0], choices: null }] }), /Invalid/, 'a history read with no choices');
  assert.throws(() => validateProjectDetails({ ...reply, warnings: undefined }), /Invalid/);
  const runs = [{ kind: 'text', text: 'Unlocks ', tone: 'H' }, { kind: 'link', entity: 'technology', token: 'mod_technology', databaseIndex: null, text: 'Gun Carriages', tone: 'H' }];
  const effects = { hour: 24, readHour: 24, output: { runs, refusals: { 'Scope unavailable': 1 } },
    pending: { reward: 'mod_project', ordinal: 0, options: [{ token: 'mod_project', effects: { runs: [], refusals: {} } }] }, unavailable: null, warnings: [] };
  assert.equal(validateProjectEffects(effects), effects);
  assert.throws(() => validateProjectEffects({ ...effects, output: { runs, refusals: { reason: 0 } } }), /Invalid/);
  assert.throws(() => validateProjectEffects({ ...effects, output: { text: 'Output', refusals: {} } }), /Invalid/, 'plain text is no longer the output');
  assert.throws(() => validateProjectEffects({ ...effects, pending: { ...effects.pending, options: [{ token: 'mod_project', effects: { runs: [{ kind: 'link', entity: 'technology', token: '', databaseIndex: null, text: 'x', tone: null }], refusals: {} } }] } }), /Invalid/);
  assert.throws(() => validateProjectEffects({ ...effects, pending: { ...effects.pending, options: [{ runs: [], refusals: {} }] } }), /Invalid/, 'an option without its identity');
  assert.throws(() => validateProjectEffects({ ...effects, readHour: 25 }), /Invalid/, 'a read after the hour shown');
  assert.throws(() => validateProjectEffects({ ...effects, readHour: null }), /Invalid/, 'effects with no supplying read');
});

test('pending options show only their own reward\'s effects, joined on the read, the reward and each option', () => {
  const output = (text: string) => ({ runs: [{ kind: 'text' as const, text, tone: null }], refusals: {} });
  const project = (reward: string, ordinal: number, readHour: number): ProjectRuntimeDetails => ({ ...reply.projects[0], readHour,
    pending: { label: { token: reward, name: null }, ordinal, description: null, options: [{ label: { token: 'mod_first', name: null } }, { label: { token: 'mod_second', name: null } }], unavailable: null } });
  const effects = (reward: string, ordinal: number, readHour: number, hour = readHour, tokens = ['mod_first', 'mod_second']): SpecialProjectEffects => ({ hour, readHour, output: output('Output'),
    pending: { reward, ordinal, options: tokens.map(token => ({ token, effects: output(`${reward} ${token}`) })) }, unavailable: null, warnings: [] });
  const shown = (asked: ProjectRuntimeDetails, reply: SpecialProjectEffects) => {
    const options = effectsRead(asked.readHour, asked, reply) ?? optionEffects(asked, reply);
    return typeof options === 'string' ? options : options.map(option => option.runs.map(run => run.kind === 'text' ? run.text : '').join(''));
  };
  const b = project('mod_b', 1, 30);
  assert.deepEqual(shown(b, effects('mod_b', 1, 30)), ['mod_b mod_first', 'mod_b mod_second']);
  assert.equal(shown(b, effects('mod_a', 0, 20, 24)), 'Loading effects…', 'the reply still on the earlier read waits instead of showing reward A beside reward B');
  assert.equal(shown(b, effects('mod_a', 0, 20, 30)), 'Effects were read from another reading of this project.');
  assert.equal(shown(b, effects('mod_a', 1, 30)), 'These effects are of another reward than the one pending.');
  assert.equal(shown(b, effects('mod_b', 0, 30)), 'These effects are of another reward than the one pending.', 'the same reward token at another occurrence is not this reward');
  assert.equal(shown(b, effects('mod_b', 1, 30, 30, ['mod_second', 'mod_first'])), 'These effects are of another reward than the one pending.', 'options join by their identity, never by position');
  assert.equal(shown(b, { ...effects('mod_b', 1, 30), readHour: null, output: null, pending: null, unavailable: 'Projects were not read' }), 'Projects were not read');
});

test('an iteration reply is the game\'s headed output read at that iteration\'s own hour, and names what it could not show', () => {
  const text = (value: string, tone: string | null = null) => ({ kind: 'text' as const, text: value, tone });
  const iteration = { hour: 23, heading: [text('Output(s)')], refusals: { 'special-project iteration effect scientist effects: Unknown scientist reference type': 1 },
    sections: [
      { heading: null, runs: [text('Project progress '), text('+15.00%', 'H'), text('\nNavy experience '), text('+15', 'G')] },
      { heading: [text('Equipment(s)')], runs: [{ kind: 'link' as const, entity: 'equipment' as const, token: 'mod_equipment', databaseIndex: null, text: 'Rifle', tone: null }] },
    ], noChanges: false, unavailable: null, warnings: [] };
  assert.equal(validateProjectIteration(iteration, 23), iteration);
  const unchanged = { ...iteration, heading: null, sections: [], noChanges: true, refusals: {} };
  assert.equal(validateProjectIteration(unchanged, 23), unchanged, 'an option that changes nothing says so with no output');
  assert.throws(() => validateProjectIteration({ ...iteration, noChanges: true }, 23), /Invalid/, 'no changes carries no output');
  assert.throws(() => validateProjectIteration({ ...iteration, noChanges: undefined }, 23), /Invalid/);
  assert.throws(() => validateProjectIteration(iteration, 24), /Invalid/, 'a reply read at another hour is not this iteration\'s');
  assert.throws(() => validateProjectIteration({ ...iteration, sections: [] }, 23), /Invalid/, 'no output carries no heading');
  assert.throws(() => validateProjectIteration({ ...iteration, sections: [{ heading: null, runs: [{ kind: 'link', entity: 'equipment', token: '', databaseIndex: null, text: 'Rifle', tone: null }] }] }, 23), /Invalid/);
  assert.throws(() => validateProjectIteration({ ...iteration, refusals: { reason: 0 } }, 23), /Invalid/);
  const missing = { hour: 23, heading: null, sections: [], noChanges: false, refusals: {}, unavailable: 'This iteration\'s choice was not read within a day of its hour', warnings: [] };
  assert.equal(validateProjectIteration(missing, 23), missing, 'not recorded is a named state, never an empty output');
});

test('a daily read moves only painted texts: only the open reward join is given each project read it joins on', () => {
  const next = { ...reply, hour: 48,
    scientists: [{ ...reply.scientists[0], active: 0, injuryDays: 6, injuryHour: 48, visibilityHour: 48, skills: [{ ...reply.scientists[0].skills[0], level: -2, experienceRaw: '100000' }] }],
    breakthroughs: [{ ...reply.breakthroughs[0], points: 20000, readHour: 48 }],
    programs: [{ ...reply.programs[0], readHour: 48, facility: { ...reply.programs[0].facility, speedRaw: '100000', sources: [{ ...reply.programs[0].facility.sources[0], valueRaw: '1' }] },
      assignments: [{ ...reply.programs[0].assignments[0], current: 3 }, reply.programs[0].assignments[1]] }],
    projects: [{ ...reply.projects[0], readHour: 48, historyReadHour: 47, variables: [{ name: 'mod_array^0', valueRaw: '5' }] }] } satisfies SpecialProjectDetails;
  assert.equal(detailsStructure(next), detailsStructure(reply), 'the inline details never render again for a project read alone');
  assert.equal(rewardsStructure({ ...next, projects: [{ ...next.projects[0], readHour: 24, historyReadHour: 23 }] }), rewardsStructure(reply));
  assert.notEqual(rewardsStructure(next), rewardsStructure(reply), 'the open reward join is given each read it joins on');
  assert.notEqual(rewardsStructure({ ...next, projects: [{ ...next.projects[0], readHour: 24 }] }), rewardsStructure(reply), 'and each choice-history read');
  assert.equal(panelViews[REWARDS_PANEL]!.values(next, null, 48, rewardsStructure(reply)), null, 'a later project read is never painted into the one joined');
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

const structureChanges = (product: string, replies: readonly unknown[]) => {
  const structure = readingStructure(product)!;
  let held: string | undefined, changes = 0;
  replies.forEach((data, at) => {
    const next = structure({ data, refused: null }, at);
    if (next !== held) changes++;
    held = next;
  });
  return changes;
};

test('a project read alone changes the structure only of the open reward join, never of the inline details', () => {
  const reads = Array.from({ length: 60 }, (_, at) => ({ ...reply, hour: 24 + at, projects: [{ ...reply.projects[0], readHour: 24 + at, historyReadHour: 23 + at }] }));
  assert.equal(structureChanges('special-project-details', reads), 1, 'sixty frames of new project reads update the inline details once, as they first arrive');
  assert.equal(structureChanges(REWARDS_PANEL, reads), 60, 'the open pop-up is given every read it joins on');
});

test('a changed effects, stats or pending reward reply changes its structure, and an unchanged one keeps it', () => {
  const runs = (text: string) => [{ kind: 'text', text, tone: null }];
  const effects = (text: string) => ({ runs: runs(text), refusals: {} });
  const replies: Record<string, (text: string) => unknown> = {
    'technology-effects': text => ({ hour: 1, runs: runs(text), refusals: {}, unavailable: null, warnings: [] }),
    'content-entity-stats': text => ({ kind: 'equipment', token: 'rifle', stats: [{ label: runs('Attack'), value: runs(text), source: null }], refusals: {}, unavailable: null, warnings: [] }),
    'special-project-effects': text => ({ hour: 1, readHour: 1, output: effects('output'), pending: { reward: 'mod_reward', ordinal: 0, options: [{ token: 'mod_option', effects: effects(text) }] }, unavailable: null, warnings: [] }),
  };
  for (const [product, reply] of Object.entries(replies)) {
    assert.equal(structureChanges(product, [reply('first'), reply('first'), reply('changed')]), 2, product);
  }
});
