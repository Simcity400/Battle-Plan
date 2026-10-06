import assert from 'node:assert/strict';
import test from 'node:test';
import type { EspionageLogReply, EspionageOperationRun, EspionageOperationEnd } from '../../generated/dashboard';
import { operationAt, operationChoices, operationJumpHours, operationRows, operationTooltip, preparationRatio, executionRatio, operationProgress, operationOutcome, validateEspionageLogReply } from './espionageData.ts';
import { entryAt } from '../../dashboard/reads.ts';
import { contentKey } from '../../dashboard/options.ts';

const icon = 'data:image/png;base64,YQ==';
const run = (firstHour: number, lastHour: number, ownerTag = 'SYNTHETIC_OWNER'): EspionageOperationRun => ({ firstHour, lastHour, readHour: firstHour,
  ownerTag, targetTag: 'SYNTHETIC_TARGET', typeKey: 'mod operation/été', phaseCount: 4, autoRepeat: false, durationDays: 8, assignedSlots: 3,
  preparationRatio: 35001, elapsedDays: 2, currentPhase: 0, terminal: false, autoCommence: true, startHour: 12, operatives: ['123', '456'] });
const ending = (byHour: number, lastSeen: number, reason: 'completed' | 'disappeared' = 'disappeared'): EspionageOperationEnd => ({ byHour, lastSeen, reason,
  completionHour: null, completionSequence: null, typeKey: 'mod operation/été', outcome: null, outcomeUnrecorded: 'Outcome not recorded', ownerTag: 'SYNTHETIC_OWNER', targetTag: 'SYNTHETIC_TARGET' });
const fixture = (): EspionageLogReply => ({ branchId: '1', firstHour: 10, lastHour: 30, available: [{ firstHour: 10, lastHour: 20 }, { firstHour: 25, lastHour: 30 }], readHours: [10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 25, 26, 27, 28, 29, 30], warnings: [], unavailable: null,
  completionsAvailable: [], completionsUnrecorded: 'Operation completion events not recorded',
  types: [{ key: 'mod operation/été', name: 'Opération des étoiles', icon, iconMissing: null }], operations: [{ definitionToken: 42, id: 'handle:10', firstRead: 10, startHour: 12, startKnownAt: 12,
    runs: [{ ...run(10, 11), durationDays: 0, startHour: null, elapsedDays: null }, run(12, 20), run(25, 30)], end: null }] });
const filter = { types: [], country: () => true, role: 'owner', from: null, live: true, runningOrder: 'start' } as const;

test('mod operation names and art survive content choices and tooltips without a vanilla catalogue', () => {
  const reply = validateEspionageLogReply(fixture());
  const choices = operationChoices(reply);
  assert.equal(choices[0].label, 'Opération des étoiles');
  assert.equal(choices[0].icon, icon);
  assert.equal(contentKey(choices[0].value), 'mod operation/été');
  const row = operationRows(reply, 18, filter)[0];
  const tip = operationTooltip(reply, row, tag => tag ?? 'Unknown')[0];
  assert.equal(tip.title, 'Opération des étoiles');
  assert.equal(tip.icon, icon);
  assert.equal(tip.rows.find(row => row.label === 'Overall civilian preparation')?.value, '35%');
  assert.ok(tip.rows.some(row => row.value === 'Name not recorded'));
  assert.equal(tip.rows.find(row => row.label === 'Outcome'), undefined);
  assert.equal(tip.rows.find(row => row.label === 'Execution progress')?.value, '2 / 8 days');
  assert.equal(executionRatio(row.item), .25);
  assert.equal(operationProgress(row.item), .25);
  assert.equal(preparationRatio(row.item), .35001);
  const malformedArt = fixture(); malformedArt.types[0].icon = 'file://unsupported-art';
  const refused = validateEspionageLogReply(malformedArt);
  assert.equal(refused.types[0].icon, null);
  assert.equal(refused.types[0].name, 'Opération des étoiles');
  assert.match(refused.warnings.at(-1)!, /mod operation\/été.*\(1\)/);
  const unnamed = fixture(); unnamed.types[0].name = null;
  assert.equal(operationChoices(unnamed)[0].label, 'Unknown operation type (mod operation/été)');
});

test('preparation and launch remain one operation without leaking its future start', () => {
  const reply = validateEspionageLogReply(fixture());
  const early = operationAt(reply, reply.operations[0], 11)!;
  assert.equal(early.startHour, null);
  assert.equal(early.state, 'preparing');
  assert.deepEqual(operationJumpHours(early, 11), [10]);
  assert.deepEqual(operationJumpHours(early, 11, 11), [11]);
  assert.equal(operationProgress(early), .35001);
  const launched = operationRows(reply, 18, filter);
  assert.equal(launched.length, 1);
  assert.equal(launched[0].id, operationRows(reply, 11, filter)[0].id);
  assert.equal(launched[0].item.state, 'running');
  assert.equal(launched[0].item.startHour, 12);
  assert.equal(launched[0].item.operation.end, null);
});

test('missing operation reads do not carry progress or invent endings', () => {
  const reply = validateEspionageLogReply(fixture());
  const missing = operationAt(reply, reply.operations[0], 24)!;
  assert.equal(missing.state, 'unread');
  assert.equal(missing.run, null);
  assert.equal(missing.readHour, null);
  assert.equal(preparationRatio(missing), null);
  assert.equal(operationAt(reply, reply.operations[0], 25)!.state, 'running');
  assert.equal(operationAt(reply, reply.operations[0], 9), null);
  const crossed = fixture(); crossed.operations[0].runs[1].lastHour = 24;
  assert.throws(() => validateEspionageLogReply(crossed), /unobserved/);
});

test('repeat, phases, duration and slot changes are dated values in one operation', () => {
  const reply = fixture();
  reply.readHours = [10, 12, 16, 25];
  reply.operations[0].runs = [{ ...run(10, 11), durationDays: 0, startHour: null }, run(12, 15),
    { ...run(16, 20), autoRepeat: true, phaseCount: 6, currentPhase: 1, durationDays: 9, assignedSlots: 5 }, run(25, 30)];
  validateEspionageLogReply(reply);
  const before = operationRows(reply, 15, filter);
  const after = operationRows(reply, 20, filter);
  assert.equal(before.length, 1); assert.equal(after.length, 1);
  assert.equal(before[0].id, after[0].id);
  assert.equal(before[0].hour, 12); assert.equal(after[0].hour, 16);
  const values = (row: typeof before[0]) => operationTooltip(reply, row, tag => tag ?? 'Unknown')[0].rows;
  const labels = ['Automatic repeat', 'Phases', 'Execution progress', 'Operative slots (including unassigned)'];
  assert.deepEqual(labels.map(label => values(before[0]).find(row => row.label === label)?.value), ['Disabled', 'Phase 1 of 4; phase names not recorded', '2 / 8 days', '3']);
  assert.deepEqual(labels.map(label => values(after[0]).find(row => row.label === label)?.value), ['Enabled', 'Phase 2 of 6; phase names not recorded', '2 / 9 days', '5']);
  assert.equal(after[0].item.operation.end, null);
});

test('a failed Countries or CountryLinks read leaves one running row with unknown countries', () => {
  const reply = fixture();
  reply.operations[0].runs = [run(10, 15), { ...run(16, 18), ownerTag: null, targetTag: null }, run(19, 20), run(25, 30)];
  validateEspionageLogReply(reply);
  const all = { ...filter, includeUnknownCountries: true };
  const before = operationRows(reply, 15, all);
  const missing = operationRows(reply, 18, all);
  const resumed = operationRows(reply, 19, all);
  assert.equal(missing.length, 1);
  assert.equal(missing[0].id, before[0].id); assert.equal(resumed[0].id, before[0].id);
  assert.equal(missing[0].item.state, 'running');
  assert.equal(missing[0].item.ownerTag, null); assert.equal(missing[0].item.targetTag, null);
  assert.equal(missing[0].item.operation.end, null);
  assert.equal(operationTooltip(reply, missing[0], tag => tag ?? 'Unknown country')[0].rows[0].value, 'Unknown country → Unknown country');
  assert.equal(resumed[0].item.ownerTag, 'SYNTHETIC_OWNER');
  assert.equal(operationRows(reply, 18, filter).length, 0);
});

test('a later read can establish the same operation start without backfilling earlier rows', () => {
  const reply = fixture();
  reply.operations[0].startKnownAt = 16;
  reply.operations[0].runs = [{ ...run(10, 11), durationDays: 0, startHour: null }, { ...run(12, 15), startHour: null }, run(16, 20), run(25, 30)];
  validateEspionageLogReply(reply);
  const before = operationRows(reply, 15, filter)[0];
  const known = operationRows(reply, 16, filter)[0];
  assert.equal(before.item.startHour, null);
  assert.equal(operationTooltip(reply, before, tag => tag ?? 'Unknown')[0].rows.find(row => row.label === 'Execution started')?.value, 'Not recorded');
  assert.equal(known.id, before.id);
  assert.equal(known.item.startHour, 12);
  assert.deepEqual(operationJumpHours(known.item, 16), [12]);
  assert.equal(operationTooltip(reply, known, tag => tag ?? 'Unknown')[0].rows.find(row => row.label === 'Execution started')?.value, 'Hour 12');
  assert.equal(known.item.operation.end, null);
});

test('a missing definition name is a recording gap and later lexical data is never backfilled', () => {
  const reply = fixture();
  reply.operations[0].runs[0].typeKey = null;
  validateEspionageLogReply(reply);
  const early = operationRows(reply, 11, filter)[0];
  const known = operationRows(reply, 12, filter)[0];
  assert.equal(early.item.typeKey, null);
  assert.equal(operationTooltip(reply, early, tag => tag ?? 'Unknown')[0].title, 'Operation type not recorded');
  assert.equal(operationTooltip(reply, known, tag => tag ?? 'Unknown')[0].title, 'Opération des étoiles');
  assert.equal(known.id, early.id);
  assert.ok(reply.types.every(type => !type.key.startsWith('?')));
  assert.deepEqual(operationRows(reply, 11, { ...filter, types: ['mod operation/été'] }), []);
});

test('operative slots include unassigned slots while missing assigned IDs never become zero', () => {
  const reply = fixture();
  const values = () => operationTooltip(reply, operationRows(reply, 18, filter)[0], tag => tag ?? 'Unknown')[0].rows;
  assert.equal(values().find(row => row.label === 'Operative slots (including unassigned)')?.value, '3');
  assert.equal(values().find(row => row.label === 'Assigned operatives')?.value, '2');
  assert.deepEqual(values().filter(row => row.label?.startsWith('Operative ')).map(row => [row.label, row.value]), [
    ['Operative slots (including unassigned)', '3'], ['Operative 123', 'Name not recorded'], ['Operative 456', 'Name not recorded'],
  ]);
  reply.operations[0].runs[1].operatives = [];
  assert.equal(values().find(row => row.label === 'Operative slots (including unassigned)')?.value, '3');
  assert.equal(values().find(row => row.label === 'Assigned operatives')?.value, '0');
  reply.operations[0].runs[1].operatives = null;
  assert.equal(values().find(row => row.label === 'Assigned operatives')?.value, 'Not recorded');
  assert.equal(values().filter(row => /^Operative \d/.test(row.label ?? '')).length, 0);
});

test('daily carried operation values and gap rows retain the actual supplying read date', () => {
  const reply = fixture();
  reply.lastHour = 50;
  reply.available = [{ firstHour: 10, lastHour: 20 }, { firstHour: 35, lastHour: 50 }];
  reply.readHours = [10, 35];
  reply.operations[0].runs = [run(10, 20), run(35, 50)];
  const carried = operationRows(reply, 18, filter)[0];
  assert.equal(carried.hour, 10);
  assert.equal(carried.item.readHour, 10);
  assert.equal(carried.item.run?.currentPhase, 0);
  assert.equal(carried.item.run?.preparationRatio, 35001);
  const gap = operationRows(reply, 25, filter)[0];
  assert.equal(gap.hour, 10);
  assert.equal(gap.item.readHour, null);
  assert.equal(gap.item.run, null);
  assert.equal(preparationRatio(gap.item), null);
  const resumed = operationRows(reply, 40, filter)[0];
  assert.equal(resumed.hour, 35);
  assert.equal(resumed.item.readHour, 35);
});

test('Live prioritises current operations; disappearance is dated by its confirming read with unknown outcome', () => {
  const reply = fixture();
  reply.operations.push({ definitionToken: 42, id: 'ended:10', firstRead: 10, startHour: 10, startKnownAt: 10, runs: [run(10, 15)], end: ending(16, 15) });
  const live = operationRows(reply, 18, filter);
  assert.deepEqual(live.map(row => row.id), ['handle:10', 'ended:10']);
  const done = live[1];
  assert.equal(done.hour, 16); assert.equal(done.by, true);
  assert.equal(done.item.state, 'disappeared');
  assert.deepEqual(operationJumpHours(done.item, 18), [10, 16]);
  assert.equal(operationTooltip(reply, done, tag => tag ?? 'Unknown')[0].rows.find(row => row.label === 'Outcome')?.value, 'Outcome not recorded');
});

test('row dates resolve changing country tags and names within the same operation', () => {
  const reply = fixture();
  reply.operations[0].runs = [run(10, 15, 'OLD_TAG'), run(16, 20, 'NEW_TAG')];
  const names = [{ firstHour: 10, value: 'Old country name' }, { firstHour: 16, value: 'Renamed country' }];
  const old = operationRows(reply, 14, filter)[0];
  const renamed = operationRows(reply, 18, filter)[0];
  assert.equal(old.hour, 14); assert.equal(old.item.ownerTag, 'OLD_TAG');
  assert.equal(renamed.hour, 18); assert.equal(renamed.item.ownerTag, 'NEW_TAG');
  assert.equal(old.id, renamed.id);
  const titleAt = (row: typeof old) => operationTooltip(reply, row, tag => tag === row.item.ownerTag ? entryAt(names, row.hour)!.value : 'Target')[0].rows[0].value;
  assert.equal(titleAt(old), 'Old country name → Target');
  assert.equal(titleAt(renamed), 'Renamed country → Target');
  reply.operations[0].end = { ...ending(25, 20), ownerTag: 'END_TAG' };
  assert.equal(operationRows(reply, 25, filter)[0].item.ownerTag, 'END_TAG');
});

test('operation log retains all 5000 countries and never applies a player-status filter', () => {
  const reply = fixture();
  reply.operations = Array.from({ length: 5000 }, (_, i) => ({ definitionToken: 42, id: `synthetic-${i}`, firstRead: 10, startHour: 12, startKnownAt: 12,
    runs: [run(10, 20, `Country_${i}`)], end: null }));
  const rows = operationRows(validateEspionageLogReply(reply), 18, filter);
  assert.equal(rows.length, 5000);
  assert.equal(new Set(rows.map(row => row.item.ownerTag)).size, 5000);
  assert.equal(operationRows(reply, 18, { ...filter, country: tag => tag === 'Country_4999' }).length, 1);
  reply.operations[0].runs[0].ownerTag = null;
  assert.equal(operationRows(reply, 18, { ...filter, includeUnknownCountries: true }).length, 5000);
});

test('execution uses the supplying read and native sentinels stay unknown', () => {
  const reply = fixture();
  reply.readHours = [10, 25];
  reply.operations[0].runs = [run(10, 20), { ...run(25, 30), elapsedDays: null }];
  assert.equal(operationProgress(operationAt(reply, reply.operations[0], 15)!), .25);
  assert.equal(operationProgress(operationAt(reply, reply.operations[0], 20)!), .25);
  assert.equal(operationProgress(operationAt(reply, reply.operations[0], 24)!), null);
  assert.equal(operationProgress(operationAt(reply, reply.operations[0], 25)!), null);
  assert.equal(operationTooltip(reply, operationRows(reply, 25, filter)[0], tag => tag ?? 'Unknown')[0].rows.find(row => row.label === 'Execution progress')?.value, 'Not recorded (native elapsed-day sentinel)');
});

test('one end-outcome adapter shows captured text or its reason only after the end', () => {
  const reply = fixture();
  reply.operations[0].end = ending(25, 20, 'completed');
  assert.equal(operationOutcome(operationAt(reply, reply.operations[0], 20)!), null);
  assert.equal(operationOutcome(operationAt(reply, reply.operations[0], 25)!), 'Outcome not recorded');
  reply.operations[0].end.outcome = { bonus: false, detected: true, recordedLanguage: 'synthetic_language', description: 'Synthetic captured outcome', descriptionUnrecorded: null, descriptionRequiredBytes: '26' };
  reply.operations[0].end.outcomeUnrecorded = null;
  assert.equal(operationOutcome(operationAt(validateEspionageLogReply(reply), reply.operations[0], 25)!), 'Synthetic captured outcome');
  const row = operationRows(reply, 25, filter)[0];
  assert.equal(operationTooltip(reply, row, tag => tag ?? 'Unknown')[0].rows.find(row => row.label === 'Recorded description language')?.value, 'synthetic_language');
  const malformed = fixture(); malformed.operations[0].runs[0].preparationRatio = 100001;
  assert.throws(() => validateEspionageLogReply(malformed), /malformed/);
});

test('completion-only operations retain the exact date, sequence and stored flags when text is refused', () => {
  const reply = fixture();
  reply.completionsAvailable = [{ firstHour: 25, lastHour: 30 }];
  reply.completionsUnrecorded = null;
  reply.operations = [{ definitionToken: 42, id: 'completion:18446744073709551615', firstRead: 25, startHour: null, startKnownAt: null, runs: [],
    end: { ...ending(25, 25, 'completed'), completionHour: 22, completionSequence: '18446744073709551615', outcomeUnrecorded: null,
      outcome: { bonus: true, detected: false, recordedLanguage: 'synthetic_language', description: null, descriptionUnrecorded: 'Stored effect description refused: acquisition budget exceeded', descriptionRequiredBytes: '18446744073709551615' } } }];
  const recorded = validateEspionageLogReply(reply);
  assert.equal(operationRows(recorded, 24, filter).length, 0);
  const rows = operationRows(recorded, 25, filter);
  assert.equal(rows.length, 1);
  const row = rows[0];
  assert.equal(row.hour, 22); assert.equal(row.by, false);
  assert.equal(row.item.run, null); assert.equal(row.item.state, 'completed');
  assert.equal(row.item.operation.end?.completionSequence, '18446744073709551615');
  assert.equal(row.item.operation.end?.outcome?.descriptionRequiredBytes, '18446744073709551615');
  assert.deepEqual(operationJumpHours(row.item, 25), [22]);
  const tooltip = operationTooltip(recorded, row, tag => tag ?? 'Unknown')[0];
  assert.equal(tooltip.rows.find(row => row.label === 'Completed')?.value, 'Hour 22');
  assert.equal(tooltip.rows.find(row => row.label === 'Outcome')?.value, 'Stored effect description refused: acquisition budget exceeded');
  assert.equal(tooltip.rows.find(row => row.label === 'Recorded description language')?.value, 'synthetic_language');
  assert.equal(tooltip.rows.find(row => row.label === 'Bonus result')?.value, 'Yes');
  assert.equal(tooltip.rows.find(row => row.label === 'Detected')?.value, 'No');
  assert.equal(tooltip.note, 'Ending read Hour 25. Earlier operation values not recorded.');
  assert.equal(operationRows(recorded, 25, { ...filter, from: 23 }).length, 0);
  reply.operations[0].end!.typeKey = null;
  assert.equal(operationTooltip(validateEspionageLogReply(reply), operationRows(reply, 25, filter)[0], tag => tag ?? 'Unknown')[0].title, 'Operation type not recorded');
  reply.completionsAvailable = [];
  assert.throws(() => validateEspionageLogReply(reply), /unrecorded completion read/);
  reply.completionsAvailable = [{ firstHour: 25, lastHour: 30 }];
  reply.operations[0].end!.completionSequence = '18446744073709551616';
  assert.throws(() => validateEspionageLogReply(reply), /malformed/);
});
