import assert from 'node:assert/strict';
import test from 'node:test';
import { cardFacets, recordingBreaks, validateOccurrence } from './scriptOccurrences.ts';
import type { EventFeedRow, FeedKindCode, ScriptEdge } from '../../generated/dashboard';

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
});

test('a card page asks the opening and continuing kinds that continue nothing, by their codes', () => {
  const kinds: FeedKindCode[] = [{ kind: 'scriptEventFired', code: 8, category: 'politics' }, { kind: 'newsEventFired', code: 9, category: 'politics' },
    { kind: 'eventOptionSelected', code: 10, category: 'politics' }, { kind: 'decisionSelected', code: 11, category: 'politics' }];
  assert.deepEqual(cardFacets(kinds, ['newsEventFired'], ['eventOptionSelected']),
    [{ name: 'kind', values: [9, 10], countries: [] }, { name: 'opener', values: [-1], countries: [] }]);
});

test('recording breaks merge gaps and event losses sorted ascending', () => {
  const breaks = recordingBreaks({ gaps: [{ firstHour: 50, lastHour: 60 }], eventLosses: [{ afterHour: 80, hour: 90 }] });
  assert.deepEqual(breaks, [{ firstHour: 50, lastHour: 60 }, { firstHour: 81, lastHour: 90 }]);
});
