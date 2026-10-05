import assert from 'node:assert/strict';
import test from 'node:test';
import type { EspionageLogReply, EspionageOperationRun } from '../../generated/dashboard';
import { operationAt, operationChoices, operationJumpHours, operationRows, operationTooltip, preparationRatio, executionRatio, operationProgress, operationOutcome, validateEspionageLogReply } from './espionageData.ts';
import { entryAt } from '../../dashboard/reads.ts';
import { contentKey } from '../../dashboard/options.ts';

const icon = 'data:image/png;base64,YQ==';
const invariants = (ownerTag = 'SYNTHETIC_OWNER') => ({ ownerTag, targetTag: 'SYNTHETIC_TARGET', typeKey: 'mod operation/été', phaseCount: 4, autoRepeat: false, durationDays: 8, assignedSlots: 3 });
const run = (firstHour: number, lastHour: number): EspionageOperationRun => ({ firstHour, lastHour, readHour: firstHour, preparationRatio: 35001, elapsedDays: 2, currentPhase: 0, terminal: false, autoCommence: true, startHour: 12, operatives: ['123', '456'] });
const fixture = (): EspionageLogReply => ({ branchId: '1', firstHour: 10, lastHour: 30, available: [{ firstHour: 10, lastHour: 20 }, { firstHour: 25, lastHour: 30 }], readHours: [10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 25, 26, 27, 28, 29, 30], warnings: [], unavailable: null,
  types: [{ key: 'mod operation/été', name: 'Opération des étoiles', icon, iconMissing: null }], operations: [{ ...invariants(), id: 'handle:10', firstRead: 10, startHour: 12, startKnownAt: 12,
    runs: [{ ...run(10, 11), startHour: null }, run(12, 20), run(25, 30)], end: null }] });
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

test('preparation does not leak a future execution start; gaps do not carry progress or invent endings', () => {
  const reply = validateEspionageLogReply(fixture());
  reply.operations[0].durationDays = 0;
  const early = operationAt(reply, reply.operations[0], 11)!;
  assert.equal(early.startHour, null);
  assert.equal(early.state, 'preparing');
  assert.deepEqual(operationJumpHours(early, 11), [10]);
  assert.deepEqual(operationJumpHours(early, 11, 11), [11]);
  assert.equal(operationProgress(early), .35001);
  reply.operations[0].durationDays = 8;
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
  reply.operations.push({ ...invariants(), id: 'ended:10', firstRead: 10, startHour: 10, startKnownAt: 10, runs: [run(10, 15)],
    end: { byHour: 16, lastSeen: 15, reason: 'disappeared', outcome: null, outcomeUnrecorded: 'Outcome not recorded', ownerTag: 'SYNTHETIC_OWNER', targetTag: 'SYNTHETIC_TARGET' } });
  const live = operationRows(reply, 18, filter);
  assert.deepEqual(live.map(row => row.id), ['handle:10', 'ended:10']);
  const done = live[1];
  assert.equal(done.hour, 16); assert.equal(done.by, true);
  assert.equal(done.item.state, 'disappeared');
  assert.deepEqual(operationJumpHours(done.item, 18), [10, 16]);
  assert.equal(operationTooltip(reply, done, tag => tag ?? 'Unknown')[0].rows.find(row => row.label === 'Outcome')?.value, 'Outcome not recorded');
});

test('row dates and episode tags keep earlier country identities apart', () => {
  const reply = fixture();
  reply.operations[0].ownerTag = 'OLD_TAG';
  reply.operations[0].runs = [run(10, 15)];
  reply.operations.push({ ...invariants('NEW_TAG'), id: 'renamed:16', firstRead: 16, startHour: 12, startKnownAt: 16, runs: [run(16, 20)], end: null });
  const names = [{ firstHour: 10, value: 'Old country name' }, { firstHour: 16, value: 'Renamed country' }];
  const old = operationRows(reply, 14, filter)[0];
  const renamed = operationRows(reply, 18, filter)[0];
  assert.equal(old.hour, 14); assert.equal(old.item.ownerTag, 'OLD_TAG');
  assert.equal(renamed.hour, 18); assert.equal(renamed.item.ownerTag, 'NEW_TAG');
  const titleAt = (row: typeof old) => operationTooltip(reply, row, tag => tag === row.item.ownerTag ? entryAt(names, row.hour)!.value : 'Target')[0].rows[0].value;
  assert.equal(titleAt(old), 'Old country name → Target');
  assert.equal(titleAt(renamed), 'Renamed country → Target');
  reply.operations[1].end = { byHour: 25, lastSeen: 20, reason: 'disappeared', outcome: null, outcomeUnrecorded: 'Outcome not recorded', ownerTag: 'END_TAG', targetTag: 'SYNTHETIC_TARGET' };
  assert.equal(operationRows(reply, 25, filter)[0].item.ownerTag, 'END_TAG');
});

test('operation log retains all 5000 countries and never applies a player-status filter', () => {
  const reply = fixture();
  reply.operations = Array.from({ length: 5000 }, (_, i) => ({ ...invariants(`Country_${i}`), id: `synthetic-${i}`, firstRead: 10, startHour: 12, startKnownAt: 12,
    runs: [run(10, 20)], end: null }));
  const rows = operationRows(validateEspionageLogReply(reply), 18, filter);
  assert.equal(rows.length, 5000);
  assert.equal(new Set(rows.map(row => row.item.ownerTag)).size, 5000);
  assert.equal(operationRows(reply, 18, { ...filter, country: tag => tag === 'Country_4999' }).length, 1);
  reply.operations[0].ownerTag = null;
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
  reply.operations[0].end = { byHour: 25, lastSeen: 20, reason: 'completed', outcome: null, outcomeUnrecorded: 'Outcome not recorded', ownerTag: 'SYNTHETIC_OWNER', targetTag: 'SYNTHETIC_TARGET' };
  assert.equal(operationOutcome(operationAt(reply, reply.operations[0], 20)!), null);
  assert.equal(operationOutcome(operationAt(reply, reply.operations[0], 25)!), 'Outcome not recorded');
  reply.operations[0].end.outcome = { description: 'Synthetic captured outcome' };
  reply.operations[0].end.outcomeUnrecorded = null;
  assert.equal(operationOutcome(operationAt(validateEspionageLogReply(reply), reply.operations[0], 25)!), 'Synthetic captured outcome');
  const malformed = fixture(); malformed.operations[0].runs[0].preparationRatio = 100001;
  assert.throws(() => validateEspionageLogReply(malformed), /malformed/);
});
