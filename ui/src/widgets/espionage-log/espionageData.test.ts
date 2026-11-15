import { test } from 'node:test';
import assert from 'node:assert/strict';
import { completionRow, endedRows, endedThrough, filteredPages, jumpHours, operationChoices, operationName, outcomeText, typeTokens, underWayRows, underWayStructure,
  validateEnded, validateOperations, validateTypes, validateTypeTokens } from './espionageData.ts';
import type { OperationFilter } from './espionageData';
import type { EndedOperation, EspionageOperationsReply, OperationState } from '../../generated/dashboard';

const state = (startHour: number | null, durationDays: number, preparationRatio: number | null, elapsedDays: number | null = null): OperationState => ({
  startHour, durationDays, phaseCount: 3, currentPhase: null, elapsedDays, preparationRatio, assignedSlots: 2, autoRepeat: false, autoCommence: true, terminal: false,
});
const country = (tag: string) => ({ id: tag.charCodeAt(0), tag });
const every: OperationFilter = { types: null, role: 'owner', country: () => true, includeUnknown: true, from: null };

test('a completion row names its countries, operation, start and outcome from its cells, a cell not recorded as not recorded', () => {
  const row = completionRow({ key: '9', hour: 100, fields: {}, cells: {
    id: { kind: 'text', value: '77' }, owner: { kind: 'number', value: '1' }, ownerTag: { kind: 'text', value: 'GER' }, target: { kind: 'number', value: '2' },
    targetTag: { kind: 'missing' }, operation: { kind: 'text', value: 'operation_steal_tech' }, completionHour: { kind: 'number', value: '99' },
    terminal: { kind: 'number', value: '1' }, bonus: { kind: 'number', value: '0' }, detected: { kind: 'number', value: '1' }, description: { kind: 'missing' },
    descriptionUnrecorded: { kind: 'text', value: 'operation completion description byte ring short' }, descriptionBytes: { kind: 'number', value: '40' },
    startHour: { kind: 'number', value: '40' }, firstSeen: { kind: 'number', value: '10' }, lastSeen: { kind: 'number', value: '98' },
  } });
  assert.deepEqual(row.item.owner, { id: 1, tag: 'GER' });
  assert.deepEqual(row.item.target, { id: 2, tag: null });
  assert.equal(row.item.startHour, 40);
  assert.equal(outcomeText(row.item), 'Description unavailable.');
  assert.deepEqual(jumpHours(row.item, row.hour), [40, 99]);
  const refused = completionRow({ ...{ key: '10', hour: 100, fields: {} }, cells: { ...{ owner: { kind: 'number', value: '1' } }, bonus: { kind: 'number', value: '2' },
    detected: { kind: 'number', value: '0' }, description: { kind: 'text', value: 'Done.' } } });
  assert.equal(outcomeText(refused.item), 'Outcome not recorded.', 'a flag byte past one is a refused outcome');
});

const reply: EspionageOperationsReply = { missing: null, warnings: [], operations: [
  { id: '1', owner: country('GER'), target: country('ENG'), operation: 'a', state: state(50, 30, 100_000, 3) },
  { id: '2', owner: country('ENG'), target: country('GER'), operation: 'b', state: state(null, 0, 20_000) },
  { id: '3', owner: country('SOV'), target: country('GER'), operation: 'a', state: state(80, 30, 100_000, 1) },
] };

test('operations under way order by start or preparation, filter by type and by the chosen role, and their numbers are not their structure', () => {
  assert.deepEqual(underWayRows(reply, 100, every, 'start').map(row => row.item.id), ['3', '1', '2']);
  assert.deepEqual(underWayRows(reply, 100, every, 'preparation').map(row => row.item.id), ['3', '1', '2']);
  const germany: OperationFilter = { ...every, country: tag => tag === 'GER' };
  assert.deepEqual(underWayRows(reply, 100, { ...germany, role: 'target' }, 'start').map(row => row.item.id), ['3', '2']);
  assert.deepEqual(underWayRows(reply, 100, { ...germany, role: 'either' }, 'start').map(row => row.item.id), ['3', '1', '2']);
  assert.deepEqual(underWayRows(reply, 100, { ...every, types: new Set(['b']) }, 'start').map(row => row.item.kind), ['preparing']);
  const moved = { ...reply, operations: reply.operations.map(operation => ({ ...operation, state: { ...operation.state, elapsedDays: 9, preparationRatio: operation.id === '2' ? 90_000 : 10_000 } })) };
  assert.equal(underWayStructure(moved, 'start'), underWayStructure(reply, 'start'), 'days and preparation are painted, never rendered');
  assert.notEqual(underWayStructure(moved, 'preparation'), underWayStructure(reply, 'preparation'), 'the preparation order is structure');
  assert.throws(() => validateOperations({ ...reply, operations: [{ ...reply.operations[0], state: { ...reply.operations[0].state, preparationRatio: 100_001 } }] }));
});

const ended = (id: string, byHour: number): EndedOperation => ({ id, owner: country('GER'), target: country('ENG'), operation: 'a', firstSeen: 10, lastSeen: byHour - 24,
  byHour, completionsRecorded: id !== '6', state: state(null, 0, 0) });

test('ended operations are cut at the head by their end and say whether completions were recorded throughout', () => {
  const endedReply = validateEnded({ operations: [ended('6', 300), ended('5', 200), ended('4', 100)], warnings: [] });
  assert.equal(endedThrough(endedReply, 99), 0);
  assert.equal(endedThrough(endedReply, 200), 2);
  assert.deepEqual(endedRows(endedReply, 2, every).map(row => row.item.id), ['5', '4']);
  assert.deepEqual(endedRows(endedReply, null, { ...every, from: 150 }).map(row => row.item.id), ['6', '5']);
  assert.equal(outcomeText(endedRows(endedReply, null, every)[0].item), 'Completion not recorded.');
  assert.equal(outcomeText(endedRows(endedReply, null, every)[1].item), 'Outcome not recorded.');
  assert.throws(() => validateEnded({ operations: [{ ...ended('1', 100), lastSeen: 100 }], warnings: [] }), 'a last sighting at or after its end is malformed');
});

test('type choices are the content’s types the recording tokenised; a chosen type never recorded filters to nothing', () => {
  const types = validateTypes({ missing: null, warnings: [], types: [
    { key: 'operation_b', name: 'Beta', icon: null, missing: null },
    { key: 'operation_a', name: null, icon: null, missing: 'Recorded content does not define operation operation_a.' },
  ] });
  const tokens = validateTypeTokens({ missing: null, warnings: [], types: [{ key: 'operation_b', tokens: [7, 9] }, { key: 'operation_a', tokens: [] }] });
  assert.deepEqual(operationChoices(types, tokens).map(choice => choice.label), ['Beta'], 'a type the recording never tokenised is not offered');
  assert.deepEqual(operationChoices(types, undefined), [], 'nor any type before the tokens are read');
  assert.equal(operationName(types.types[1]), 'Operation name unavailable', 'a type the content does not name is never shown by its key');
  assert.equal(operationName(undefined), 'Operation name unavailable', 'nor is an operation of a type the reply does not hold');
  assert.equal(operationName(undefined, true), 'Loading name…', 'while the types load, it says so');
  assert.equal(operationName(types.types[0]), 'Beta', 'a name is read from the types alone, never waiting on the tokens');
  assert.equal(typeTokens(tokens, null), null);
  assert.equal(typeTokens(undefined, new Set(['operation_b'])), undefined, 'a chosen type waits for its tokens, never reads as none recorded');
  assert.deepEqual(typeTokens(tokens, new Set(['operation_b'])), [7, 9]);
  assert.deepEqual(typeTokens(tokens, new Set(['operation_a'])), []);
  assert.throws(() => validateTypeTokens({ missing: null, warnings: [], types: [{ key: 'operation_b', tokens: [-1] }] }));
});

test('pages the server could not filter by country keep only matching rows and their counts', () => {
  const rows = [completionRow({ key: '1', hour: 5, fields: {}, cells: { owner: { kind: 'number', value: '1' }, ownerTag: { kind: 'text', value: 'GER' } } }),
    completionRow({ key: '2', hour: 6, fields: {}, cells: { owner: { kind: 'number', value: '2' }, ownerTag: { kind: 'text', value: 'ENG' } } })];
  const pages = [{ key: '0', start: 0, end: 2, count: 2, rows }];
  const kept = filteredPages(pages, { ...every, includeUnknown: false, country: tag => tag === 'ENG' });
  assert.deepEqual(kept[0].rows?.map(row => row.key), ['2']);
  assert.equal(kept[0].count, 2);
  assert.equal(filteredPages(pages, null), pages);
});

