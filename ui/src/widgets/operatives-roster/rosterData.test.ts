import { test } from 'node:test';
import assert from 'node:assert/strict';
import { capturedText, operativeMoments, operativeName, paintedRows, paintedText, rosterRows, rosterStructure, validateDeparted, validateRoster, validateTenures } from './rosterData.ts';
import type { OperativesRosterReply, RosterOperative } from '../../generated/dashboard';

const operative = (fields: Partial<RosterOperative> = {}): RosterOperative => ({
  id: '42', roster: 'GER', owner: { id: 1, tag: 'GER' }, name: 'Hanna Mohrlang', unnamed: false, nameMissing: null, portrait: 0, portraitMissing: null,
  skill: 2, experienceRaw: '150000', experienceText: 'Skill 2 (50%)', experienceRatio: 0.5, experienceMissing: null, assignment: 3,
  assignmentName: 'Build intel network', assignmentDuration: null, assignmentProgress: null, assignmentMissing: null, assignmentIcon: 1,
  assignmentIconMissing: null, target: { id: 2, tag: 'ENG' }, captured: false, capturer: null, capturedHour: null, ...fields,
});
const reply = (operatives: RosterOperative[]): OperativesRosterReply => ({
  operatives, artwork: ['data:image/png;base64,AA', 'data:image/png;base64,BB'], unheld: [], agencyFeature: true, readHour: 60_800_000, missing: null, warnings: [],
});

test('a roster names its art by index and refuses an index past it', () => {
  const ok = reply([operative()]);
  assert.equal(validateRoster(ok), ok);
  assert.throws(() => validateRoster(reply([operative({ portrait: 2 })])));
  assert.throws(() => validateRoster(reply([operative({ unnamed: true })])), 'an unnamed operative has no name');
});

test('several agencies make one table, and a group member only while it is one', () => {
  const roster = reply([operative(), operative({ id: '7', roster: 'ITA', owner: { id: 3, tag: 'ITA' } })]);
  const tenures = validateTenures({ firstHour: 60_000_000, lastHour: 61_000_000, tenures: [{ id: '42', roster: 'GER', firstHour: 60_500_000, lastHour: 61_000_000 }], warnings: [] });
  const rows = rosterRows(roster, undefined, tenures, 60_800_000, tag => tag !== 'ITA');
  assert.deepEqual(rows.map(row => row.operative.id), ['42']);
  assert.equal(rows[0]!.joinedHour, 60_500_000);
  assert.deepEqual(operativeMoments(rows[0]!).map(moment => moment.kind), ['joined']);
});

test('departed operatives come after the list, dated and muted by their reason', () => {
  const departed = validateDeparted({ departed: [{ ...operative({ id: '9', assignment: 5 }), firstHour: 60_100_000, lastHour: 60_200_000, departedHour: 60_200_001, departedReason: 'Killed' }],
    artwork: ['data:image/png;base64,CC', 'data:image/png;base64,DD'], warnings: [] });
  const rows = rosterRows(reply([operative()]), departed, undefined, 60_800_000);
  assert.deepEqual(rows.map(row => [row.operative.id, row.departed?.departedReason ?? null]), [['42', null], ['9', 'Killed']]);
  assert.deepEqual(operativeMoments(rows[1]!).map(moment => moment.kind), ['joined', 'departed']);
  assert.throws(() => validateDeparted({ departed: [{ ...operative(), firstHour: 10, lastHour: 20, departedHour: 20, departedReason: 'Killed' }], artwork: ['x', 'y'], warnings: [] }));
});

test('the census moving experience and days changes no structure; a new assignment does', () => {
  const before = reply([operative({ assignment: 1, assignmentName: 'Forced into hiding', assignmentDuration: '3 days', assignmentProgress: 0.25 })]);
  const later = reply([operative({ assignment: 1, assignmentName: 'Forced into hiding', assignmentDuration: '2 days', assignmentProgress: 0.5, experienceRaw: '170000', experienceText: 'Skill 2 (57%)', experienceRatio: 0.57 })]);
  assert.equal(rosterStructure(before), rosterStructure(later));
  assert.notEqual(rosterStructure(before), rosterStructure(reply([operative()])));
  const painted = paintedRows(later).get(JSON.stringify(['GER', '42']));
  assert.equal(paintedText(painted, 'duration'), '2 days');
  assert.equal(paintedText(painted, 'experience'), 'Skill 2 (57%)');
  assert.equal(paintedText(painted, 'progress'), '50%');
});

test('a name the game saved empty is unnamed; a capture says by whom, or that it is unknown', () => {
  assert.equal(operativeName(operative({ name: null, unnamed: true })), 'Unnamed operative');
  assert.equal(operativeName(operative({ name: null, nameMissing: 'not recorded' })), 'Name unknown');
  assert.match(capturedText(operative({ captured: true, capturer: { id: 2, tag: 'ENG' }, capturedHour: 60_800_000 }), () => 'United Kingdom'), /^United Kingdom · /);
  assert.match(capturedText(operative({ captured: true, capturer: { id: 0, tag: null } }), () => 'x'), /^Yes · capturer unknown · Capture date unknown$/);
  assert.equal(capturedText(operative(), () => 'x'), 'No');
});
