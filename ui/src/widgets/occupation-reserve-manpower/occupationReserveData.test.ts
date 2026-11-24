import { test } from 'node:test';
import assert from 'node:assert/strict';
import { liveKey, valuesOf } from '../../charts/live.ts';
import type { OccupationReserveReply, ReserveHistoryReply } from '../../generated/dashboard';
import { holderBands, historySources, holderTooltip, raceEntries, raceHolders, reserveOf, reserveStructure, segmentParts, sourceParts, sourceTagAt, validateOccupationReserve, validateReserveHistory } from './occupationReserveData.ts';

const country = (id: number, tag: string | null = `T${id}`) => ({ id, tag });
const reply: OccupationReserveReply = { holders: [
  { country: country(1), admitted: true, entries: [] },
  { country: country(2), admitted: true, entries: [{ ordinal: 0, source: country(8) }, { ordinal: 1, source: country(9) }, { ordinal: 2, source: country(9) }] },
  { country: country(3), admitted: false, entries: [{ ordinal: 0, source: country(9, null) }] },
  { country: country(4), admitted: false, entries: [] },
  { country: country(5, null), admitted: true, entries: [] },
], readHour: 100, missing: null, warnings: [] };
const values = (cells: Record<string, number>) => valuesOf(new Map(Object.entries(cells).map(([key, value]) => {
  const [row, column] = key.split('/');
  return [liveKey(row, column), value];
})), 'Not read yet');
const amounts = values({ '2:0/amount': -50, '2:1/amount': 300, '2:2/amount': 200, '3:0/amount': 700 });

test('a holder holds the sum of its positive entries, by source largest first; a negative one is kept apart, signed', () => {
  const reserve = reserveOf(reply.holders[1], amounts);
  assert.equal(reserve.positive, 500);
  assert.deepEqual(sourceParts(reserve), [{ source: 'T9', amount: 500 }]);
  assert.deepEqual(reserve.negative.map(entry => entry.amount), [-50]);
  assert.equal(reserveOf(reply.holders[1], values({ '2:0/amount': 5 })).positive, null);
  assert.equal(reserveOf(reply.holders[0], amounts).positive, 0);
});

test('the race shows holders admitted or holding entries that a tag names, most held first, the top N when asked', () => {
  const shown = raceHolders(reply, () => true);
  assert.deepEqual(shown.map(holder => holder.country.id), [1, 2, 3]);
  const race = raceEntries(shown, amounts, null, source => `c-${source}`, 10);
  assert.deepEqual(race.map(entry => [entry.tag, entry.value]), [['T3', 700], ['T2', 500], ['T1', 0]]);
  assert.deepEqual(race[0].parts, [{ id: 'unresolved', value: 700, color: 'c-unresolved' }]);
  assert.deepEqual(raceEntries(shown, amounts, 1, () => '', 10).map(entry => entry.tag), ['T3']);
  assert.deepEqual(raceHolders(reply, tag => tag === 'T2').map(holder => holder.country.id), [2]);
});

test('no list of zeros: a top N ranks only holders above zero, and with no entry anywhere nothing is listed', () => {
  assert.deepEqual(raceHolders(reply, () => true, true).map(holder => holder.country.id), [2, 3], 'a holder with no entry holds a proven zero');
  const zero = values({ '2:0/amount': 0, '2:1/amount': 0, '2:2/amount': 0, '3:0/amount': 700 });
  assert.deepEqual(raceEntries(raceHolders(reply, () => true, true), zero, 10, () => '', 10).map(entry => entry.tag), ['T3']);
  const unread = values({ '3:0/amount': 700 });
  assert.deepEqual(raceEntries(raceHolders(reply, () => true, true), unread, 10, () => '', 10).map(entry => [entry.tag, entry.label]), [['T3', '700'], ['T2', 'Not read']],
    'an amount not read is never taken for a zero');
  const none: OccupationReserveReply = { ...reply, holders: reply.holders.map(holder => ({ ...holder, entries: [] })) };
  assert.deepEqual(raceHolders(none, () => true), [], 'every country at zero is said as none held, not listed in tag order');
});

test('amounts are not structure: a moving reserve renders nothing, a source arriving does', () => {
  assert.equal(reserveStructure(reply), reserveStructure(validateOccupationReserve(JSON.parse(JSON.stringify({ ...reply, readHour: 124 })))));
  const more = { ...reply, holders: [...reply.holders.slice(0, 4), { ...reply.holders[4], entries: [{ ordinal: 0, source: country(8) }] }] };
  assert.notEqual(reserveStructure(reply), reserveStructure(more));
});

test('the tooltip lists each entry, the negative ones apart, and the read date', () => {
  const tip = holderTooltip(reserveOf(reply.holders[1], amounts), 'Germany', source => source.tag ?? 'Source country unavailable', () => 'red', '1 Jan 1940');
  assert.deepEqual(tip.rows.map(row => [row.label, row.value]), [['Reserve manpower held', '500'], ['T9', '300'], ['T9', '200'], ['Negative entries', undefined],
    ['T8', '-50'], ['As of', '1 Jan 1940']]);
});

const history: ReserveHistoryReply = {
  segments: [
    { firstHour: 0, lastHour: 9, admitted: true, entries: [{ source: 8, amount: 100 }], unknown: null },
    { firstHour: 10, lastHour: 19, admitted: true, entries: [{ source: 8, amount: 100 }, { source: 9, amount: 400 }, { source: 9, amount: -20 }], unknown: null },
    { firstHour: 20, lastHour: 29, admitted: false, entries: null, unknown: 'The reserve was not recorded at these hours' },
    { firstHour: 40, lastHour: 49, admitted: true, entries: [], unknown: null },
  ],
  sources: [{ id: 8, tags: [{ tag: 'GER', firstHour: 0, lastHour: 49 }] }, { id: 9, tags: [{ tag: null, firstHour: 10, lastHour: 14 }, { tag: 'ITA', firstHour: 15, lastHour: 19 }] }],
  warnings: [],
};

test('the holder drill stacks each source over time, an admitted holder at zero from a source it has none from, unknown hours a gap', () => {
  assert.equal(validateReserveHistory(history), history);
  const sources = historySources(history);
  assert.deepEqual(sources, [9, 8]);
  assert.deepEqual(segmentParts(history.segments[0], sources), [0, 100]);
  assert.deepEqual(segmentParts(history.segments[2], sources), [undefined, undefined]);
  const bands = holderBands(history, sources);
  assert.deepEqual(bands[0], [{ firstHour: 0, lastHour: 9, value: 0, breakBefore: true }, { firstHour: 10, lastHour: 19, value: 400, breakBefore: false },
    { firstHour: 40, lastHour: 49, value: 0, breakBefore: true }]);
  assert.deepEqual(bands[1], [{ firstHour: 0, lastHour: 19, value: 100, breakBefore: true }, { firstHour: 40, lastHour: 49, value: 0, breakBefore: true }]);
  assert.equal(sourceTagAt(history, 9, 12), null);
  assert.equal(sourceTagAt(history, 9, 16), 'ITA');
  assert.throws(() => validateReserveHistory({ ...history, segments: [history.segments[1], history.segments[0]] }));
});

