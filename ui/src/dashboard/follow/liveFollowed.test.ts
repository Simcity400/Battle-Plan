import assert from 'node:assert/strict';
import test from 'node:test';
import { answerValues } from './liveFollowed.ts';
import { CELL, parseFollowed } from './message.ts';
import { message } from './messages.fixture.ts';

const answer = (spec: Parameters<typeof message>[1][number]['answer']) => parseFollowed(message(10, [{ id: 1, answer: spec }])).answers[0]!.answer;

test('a followed answer gives each asked key its value, and a key without one is missing for its reason, never zero', () => {
  const rows = answer({ keys: [1n, 2n, 3n], columns: 1, values: [12_50n, 0n, 0n], states: [CELL.value, CELL.unrecorded, CELL.value], reasons: [null, 'the read failed', null] });
  const values = answerValues(rows, ['GER', 'SOV', 'ITA'], 2);
  assert.equal(values.get('GER'), 12.5);
  assert.equal(values.get('SOV'), undefined);
  assert.equal(values.missing('SOV'), 'the read failed');
  assert.equal(values.get('ITA'), 0, 'a recorded zero is a value');
  assert.equal(values.get('JAP'), undefined);
  assert.equal(values.missing('JAP'), 'Not in the answer');
  assert.equal(answerValues(undefined, ['GER']).missing('GER'), 'Not read yet');
  const refused = answerValues(answer({ notRecorded: 'not recorded in this campaign' }), ['GER']);
  assert.equal(refused.get('GER'), undefined);
  assert.equal(refused.missing('GER'), 'not recorded in this campaign');
});

