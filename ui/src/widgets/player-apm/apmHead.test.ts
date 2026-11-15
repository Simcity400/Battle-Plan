import assert from 'node:assert/strict';
import test from 'node:test';
import { liveKey } from '../../charts/live.ts';
import { CELL, parseFollowed } from '../../dashboard/follow/message.ts';
import { message } from '../../dashboard/follow/messages.fixture.ts';
import type { Spec } from '../../dashboard/follow/messages.fixture.ts';
import { apmFromSort, apmHeadRows, apmHeadStructure, apmHolders, apmLiveValues, apmPausedView, apmSortValue, clockHead } from './apmHead.ts';

const bits = (value: number) => { const view = new DataView(new ArrayBuffer(8)); view.setFloat64(0, value, true); return view.getBigInt64(0, true); };
const packed = (tag: string) => [...tag].reduce((out, char, at) => out | BigInt(char.charCodeAt(0)) << BigInt(8 * at), 0n);
const answer = (spec: Spec) => parseFollowed(message(100, [{ id: 1, answer: spec }])).answers[0]!.answer;

test('a followed eAPM reads back as the request path\'s series: rates exactly, counts, tags, a missing value as missing with its reason', () => {
  const V = CELL.value, M = CELL.missing;
  const followed = answer({ keys: [0n, 1n], columns: 9, names: ['GER', 'user:ab'],
    values: [bits(2.35), bits(2.35), 47n, 3n, bits(20), 1n, 0n, packed('GER'), 160n,
      0n, bits(0.5), 0n, 0n, bits(0), 1n, 1n, packed('ITA'), 150n],
    states: [V, V, V, V, V, V, V, V, V, M, V, M, M, V, V, V, V, V] });
  const rows = apmHeadRows(followed);
  assert.deepEqual(rows[0], { id: 'GER', name: 'GER', tag: 'GER', hour: 160, points: [],
    summary: { apm: 2.35, confirmedApm: 2.35, orders: '47', pausedOrders: '3', unpausedMinutes: 20, complete: true, gaps: false } });
  assert.deepEqual(rows[1]!.summary, { apm: null, confirmedApm: 0.5, orders: null, pausedOrders: null, unpausedMinutes: 0, complete: true, gaps: true });
  assert.equal(rows[1]!.tag, 'ITA');
  assert.equal(apmHeadStructure(followed), 'GER:GER\u0000user:ab:ITA\u0000');
  const live = apmLiveValues(followed);
  assert.deepEqual(apmFromSort(live.get(liveKey('GER', 'apm')) as bigint), { apm: 2.35, gaps: false });
  assert.equal(live.get(liveKey('GER', 'orders')), 47n);
  assert.equal(live.get(liveKey('user:ab', 'apm')), undefined, 'an unknown rate is missing, never zero');
  assert.equal(live.missing(liveKey('user:ab', 'apm')), 'No unpaused time was observed in this period.');
  assert.equal(live.missing(liveKey('SOV', 'apm')), 'Not in the period');
  assert.ok((apmSortValue({ ...rows[0]!.summary, gaps: true }) as bigint) > (apmSortValue(rows[0]!.summary) as bigint), 'gaps ride below the rate, which orders the sort');
  assert.deepEqual(apmFromSort(apmSortValue({ ...rows[0]!.summary, apm: 7.1, gaps: true })!), { apm: 7.1, gaps: true });
  const refused = answer({ notRecorded: 'Player orders were not recorded in this campaign.' });
  assert.deepEqual(apmHeadRows(refused), []);
  assert.equal(apmLiveValues(refused).missing(liveKey('GER', 'apm')), 'Player orders were not recorded in this campaign.');
});

test('the followed clock reads its seconds exactly, an unclassified playing time as missing, a refusal as its reason', () => {
  const V = CELL.value, M = CELL.missing;
  assert.deepEqual(clockHead(answer({ keys: [0n], columns: 4, names: ['clock'], values: [bits(3723.25), bits(3000.5), bits(722.75), 1n], states: [V, V, V, V] })),
    { recorded: 3723.25, playing: 3000.5, paused: 722.75, complete: true });
  assert.deepEqual(clockHead(answer({ keys: [0n], columns: 4, names: ['clock'], values: [bits(10), 0n, 0n, 0n], states: [V, M, M, V] })),
    { recorded: 10, playing: null, paused: null, complete: false });
  assert.deepEqual(clockHead(answer({ notRecorded: 'Playing and paused time were not recorded in this campaign.' })), { reason: 'Playing and paused time were not recorded in this campaign.' });
  assert.equal(clockHead(undefined), null);
});

test("the graph's legend names each holder as at the hour shown while it follows the head, never as at the campaign's end", () => {
  // The branch's series, asked over the whole branch, carry each holder's last hour: the campaign's end for Germany.
  const rows = [{ id: 'GER@1', tag: 'GER', hour: 100_000 }, { id: 'ETH@2', tag: 'ETH', hour: 3_000 }, { id: 'user:ab', tag: null, hour: 100_000 }];
  assert.deepEqual(apmHolders(rows, null), [{ key: 'GER@1', tag: 'GER', hour: 100_000 }, { key: 'ETH@2', tag: 'ETH', hour: 3_000 }]);
  // At the head in 1936 (hour 2,000): Germany is named there; a holder whose series ended earlier at its own last hour.
  assert.deepEqual(apmHolders(rows, 2_000), [{ key: 'GER@1', tag: 'GER', hour: 2_000 }, { key: 'ETH@2', tag: 'ETH', hour: 2_000 }]);
  assert.deepEqual(apmHolders(rows, 50_000), [{ key: 'GER@1', tag: 'GER', hour: 50_000 }, { key: 'ETH@2', tag: 'ETH', hour: 3_000 }]);
});

test('paused or after a seek the graph view ends at the head, never past it; while playing the chart window follows', () => {
  const first = 1000, last = 9000, week = 168;
  assert.deepEqual(apmPausedView(first, first, last, week), [first, first + 1], 'at the first hour, its one hour: never two weeks ahead');
  assert.deepEqual(apmPausedView(first + 50.6, first, last, week), [first, first + 50]);
  assert.deepEqual(apmPausedView(first + 4000, first, last, week), [first + 4000 - week, first + 4000]);
  assert.deepEqual(apmPausedView(last + 30, first, last, week), [last - week, last], 'a head past the branch ends at its last hour');
  assert.equal(apmPausedView(null, first, last, week), null);
});
