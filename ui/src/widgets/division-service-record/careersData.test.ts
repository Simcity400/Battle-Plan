import assert from 'node:assert/strict';
import test from 'node:test';
import type { CareerFacts, DivisionRecordReply, RecordEntry } from '../../generated/dashboard';
import { experienceRatio, headerDetail, recordRows, validateDivisionRecord } from './careersData.ts';

const facts = (extra: Partial<CareerFacts> = {}): CareerFacts => ({ medals: 3, awards: [], historyMissing: null, experienceRaw: '60000', level: 2, levelName: 'Veteran', kills: 1234, ...extra });
const entry = (award: boolean, ordinal: number, hour: number): RecordEntry => ({ award, ordinal, hour, description: `E${ordinal}`, descriptionMissing: null, provinces: [], medal: award ? 0 : null });
const reply = (entries: RecordEntry[]): DivisionRecordReply => validateDivisionRecord({ branchId: '1', hour: 100, division: '7', absent: false, tag: 'GER', template: null,
  name: 'First', nameMissing: null, facts: facts(), medals: [{ key: 'm', name: 'Medal' }], entries, warnings: [] });

test('the record lists entries newest first and filters by the Show setting', () => {
  const rows = recordRows(reply([entry(false, 0, 10), entry(true, 0, 30), entry(false, 1, 20), entry(true, 1, 30)]), 'both');
  assert.deepEqual(rows.map(row => row.key), ['award:1', 'award:0', 'career:1', 'career:0']);
  assert.equal(rows[0].item.medal?.name, 'Medal');
  assert.deepEqual(recordRows(reply([entry(false, 0, 10), entry(true, 0, 30)]), 'career').map(row => row.key), ['career:0']);
  assert.deepEqual(recordRows(reply([entry(false, 0, 10), entry(true, 0, 30)]), 'awards').map(row => row.key), ['award:0']);
});

test('header parts are missing on their own, never zero', () => {
  assert.equal(headerDetail('Infantry', facts()), 'Infantry · 3 medals · Veteran · 1,234 enemy soldiers killed');
  assert.equal(headerDetail(null, facts({ medals: null, kills: null, levelName: null })), '— · — · — · —');
  assert.equal(headerDetail('Infantry', null), 'Infantry · — · — · —');
});

test('experience shows only as a share of the full scale', () => {
  assert.equal(experienceRatio(facts()), 0.6);
  assert.equal(experienceRatio(facts({ experienceRaw: null })), null);
  assert.equal(experienceRatio(facts({ experienceRaw: '4294967295' })), null, 'the arithmetic sentinel is not drawn');
});
