import assert from 'node:assert/strict';
import test from 'node:test';
import type { LogPage, LogRow } from '../../charts/log/logData';
import { cardFacets, cardsKey, cutPages, occurrenceOf, recordingBreaks } from './scriptOccurrences.ts';
import type { EventFeedRow, FeedKindCode, ScriptEdge } from '../../generated/dashboard';

const edge = (kind: EventFeedRow['kind'], hour: number, extra: Partial<ScriptEdge> = {}): ScriptEdge => ({
  row: { id: `native:1:${hour}`, hour, readHour: hour, nameHour: hour, exact: true, category: 'politics', kind, tags: ['GER'], participants: ['GER', null],
    item: null, amount: null }, option: null, lifecycle: null, days: null, costs: null, ...extra });

test('a card page row is its occurrence, every edge a checked feed row with its operands; a row that is no card is null', () => {
  const occurrence = { id: 'native:1:5', opened: true, edges: [edge('decisionSelected', 5, { costs: [{ currency: 'politicalPower', amount: '-5000000' }] }),
    edge('decisionCompleted', 9)] };
  assert.deepEqual(occurrenceOf({ key: '1', hour: 5, cells: {}, fields: { occurrence } }), occurrence);
  assert.equal(occurrenceOf({ key: '1', hour: 5, cells: {}, fields: { occurrence: null } }), null);
  assert.throws(() => occurrenceOf({ key: '1', hour: 5, cells: {}, fields: { occurrence: { ...occurrence, edges: [] } } }), /malformed/);
  assert.throws(() => occurrenceOf({ key: '1', hour: 5, cells: {}, fields: { occurrence: { ...occurrence,
    edges: [edge('decisionSelected', 5, { costs: [{ currency: 'gold' as never, amount: '1' }] })] } } }), /malformed/, 'a currency the game has not');
});

test('a card page asks the opening and continuing kinds that continue nothing, by their codes', () => {
  const kinds: FeedKindCode[] = [{ kind: 'scriptEventFired', code: 8, category: 'politics' }, { kind: 'newsEventFired', code: 9, category: 'politics' },
    { kind: 'eventOptionSelected', code: 10, category: 'politics' }, { kind: 'decisionSelected', code: 11, category: 'politics' }];
  assert.deepEqual(cardFacets(kinds, ['newsEventFired'], ['eventOptionSelected']),
    [{ name: 'kind', values: [9, 10], countries: [] }, { name: 'opener', values: [-1], countries: [] }]);
});

test('the hour a card list reads moves only where a held card changes', () => {
  const breaks = recordingBreaks({ gaps: [{ firstHour: 50, lastHour: 60 }], eventLosses: [{ afterHour: 80, hour: 90 }] });
  assert.deepEqual(breaks, [{ firstHour: 50, lastHour: 60 }, { firstHour: 81, lastHour: 90 }]);
  const key = (hour: number, days = false) => cardsKey([10, 30], breaks, days, hour);
  assert.equal(key(10), key(29), 'nothing changes between two edges');
  assert.notEqual(key(29), key(30));
  assert.notEqual(key(49), key(50), 'a break begins');
  assert.notEqual(key(23, true), key(24, true), 'an open card names its days');
});

test('a loaded page holds the cards the hour shows, counted by them; a page not loaded keeps its count', () => {
  const rows: LogRow<number>[] = [1, 2, 3].map(value => ({ key: String(value), hour: value, item: value }));
  const pages: LogPage<number>[] = [{ key: '0', start: 0, end: 2, count: 3, rows }, { key: '1', start: 3, end: 502, count: 500 }];
  const cut = cutPages(pages, row => row.item % 2 ? { ...row, item: `card ${row.item}` } : null);
  assert.deepEqual(cut.map(page => [page.count, page.rows?.map(row => row.item) ?? null]), [[2, ['card 1', 'card 3']], [500, null]]);
});
