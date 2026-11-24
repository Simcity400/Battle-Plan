import assert from 'node:assert/strict';
import test from 'node:test';
import { headerDetail, recordRows, topDivision, validateDivisionRecord } from './careersData.ts';
import type { CareerFacts, CareerRow, DivisionRecordReply, RecordEntry } from '../../generated/dashboard';

const facts = (extra: Partial<CareerFacts> = {}): CareerFacts => ({ medals: 3, awards: [], historyMissing: null, experienceRaw: '60000', level: 2, levelName: 'Veteran', casualtiesTaken: 1234, ...extra });
const entry = (award: boolean, ordinal: number, hour: number): RecordEntry => ({ award, ordinal, hour, description: `E${ordinal}`, descriptionMissing: null, provinces: [], medal: award ? 0 : null });
const reply = (entries: RecordEntry[]): DivisionRecordReply => validateDivisionRecord({ division: '7', absent: false, tag: 'GER', template: null,
  name: 'First', nameMissing: null, facts: facts(), medals: [{ key: 'm', name: 'Medal' }], entries, warnings: [] });

test('the record lists entries newest first and filters by the Show setting', () => {
  const rows = recordRows(reply([entry(false, 0, 10), entry(true, 0, 30), entry(false, 1, 20), entry(true, 1, 30)]), 'both');
  assert.deepEqual(rows.map(row => row.key), ['award:1', 'award:0', 'career:1', 'career:0']);
  assert.equal(rows[0].item.medal?.name, 'Medal');
  assert.deepEqual(recordRows(reply([entry(false, 0, 10), entry(true, 0, 30)]), 'career').map(row => row.key), ['career:0']);
  assert.deepEqual(recordRows(reply([entry(false, 0, 10), entry(true, 0, 30)]), 'awards').map(row => row.key), ['award:0']);
});

test('header parts are missing on their own, never zero', () => {
  assert.equal(headerDetail('Infantry', facts()), 'Infantry · 3 medals · Veteran · 1,234 casualties taken');
  assert.equal(headerDetail(null, facts({ medals: null, casualtiesTaken: null, levelName: null })), '— · — · — · —');
  assert.equal(headerDetail('Infantry', null), 'Infantry · — · — · —');
});

test('the automatic record follows Most decorated divisions’ first row, only when it holds a medal', () => {
  const row = (division: string, medals: number | null): CareerRow => ({ division, tag: 'GER', template: '1', name: division, nameMissing: null, facts: facts({ medals }) });
  assert.equal(topDivision([row('9', 1), row('12', 2), row('10', 2)])?.division, '10', 'most medals first, ties by division as the table breaks them');
  assert.equal(topDivision([row('1', 0), row('2', null)]), null, 'no medal, or medals not recorded, is never most decorated');
  assert.equal(topDivision([]), null);
});
