import assert from 'node:assert/strict';
import test from 'node:test';
import { recordingBreaks, validateOccurrence } from './scriptOccurrences.ts';
import type { EventFeedRow, ScriptEdge } from '../../generated/dashboard';

const edge = (kind: EventFeedRow['kind'], hour: number, extra: Partial<ScriptEdge> = {}): ScriptEdge => ({
  row: { id: `native:1:${hour}`, hour, readHour: hour, nameHour: hour, exact: true, category: 'politics', kind, tags: ['GER'], participants: ['GER', null],
    item: null, amount: null }, option: null, lifecycle: null, days: null, costs: null, ...extra });

test('an occurrence validates every edge as a checked feed row with its operands', () => {
  const occurrence = { id: 'native:1:5', opened: true, edges: [edge('decisionSelected', 5, { costs: [{ currency: 'politicalPower', amount: '-5000000' }] }),
    edge('decisionCompleted', 9)] };
  assert.deepEqual(validateOccurrence(occurrence), occurrence);
  assert.throws(() => validateOccurrence({ ...occurrence, edges: [] }), /malformed/);
  assert.throws(() => validateOccurrence({ ...occurrence,
    edges: [edge('decisionSelected', 5, { costs: [{ currency: 'gold' as never, amount: '1' }] })] }), /malformed/, 'a currency the game has not');
  const { option: _option, lifecycle: _lifecycle, days: _days, costs: _costs, ...bare } = edge('decisionCompleted', 9);
  const { amount: _amount, ...row } = bare.row;
  const sparse = { id: 'native:1:9', opened: true, edges: [{ ...bare, row: { ...row, item: { token: 'd', name: 'D', icon: null } } }] };
  assert.deepEqual(validateOccurrence(sparse), sparse, 'a reply leaves out what it does not hold: an edge with no option, lifecycle, days or costs, a row with no amount, an item with no description');
});

test('recording breaks merge gaps and event losses sorted ascending', () => {
  const breaks = recordingBreaks({ gaps: [{ firstHour: 50, lastHour: 60 }], eventLosses: [{ afterHour: 80, hour: 90 }] });
  assert.deepEqual(breaks, [{ firstHour: 50, lastHour: 60 }, { firstHour: 81, lastHour: 90 }]);
});
