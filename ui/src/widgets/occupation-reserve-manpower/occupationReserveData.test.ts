import { test } from 'node:test';
import assert from 'node:assert/strict';
import type { OccupationReserveReply, ReserveHistoryReply } from '../../generated/dashboard';
import { holderBands, historySources, segmentParts, sourceTagAt, validateOccupationReserve, validateReserveHistory } from './occupationReserveData.ts';

const country = (id: number, tag: string | null = `T${id}`) => ({ id, tag });
const reply: OccupationReserveReply = {
  holders: [
    { country: country(3), admitted: false, total: 700, entries: [{ ordinal: 0, source: country(9, null), amount: 700 }] },
    { country: country(2), admitted: true, total: 500, entries: [{ ordinal: 0, source: country(8), amount: -50 }, { ordinal: 1, source: country(9), amount: 300 }, { ordinal: 2, source: country(9), amount: 200 }] },
    { country: country(1), admitted: true, total: 0, entries: [] },
  ],
  readHour: 100,
  unnamed: 1,
  missing: null,
  warnings: [],
};

test('validateOccupationReserve accepts a ranked reply with totals and amounts and rejects malformed shapes', () => {
  assert.deepEqual(validateOccupationReserve(reply), reply);
  assert.throws(() => validateOccupationReserve({ ...reply, unnamed: -1 }));
  assert.throws(() => validateOccupationReserve({ ...reply, holders: [{ country: country(1), admitted: true, entries: [] }] }));
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
  assert.deepEqual(segmentParts(history.segments[0]!, sources), [0, 100]);
  assert.deepEqual(segmentParts(history.segments[2]!, sources), [undefined, undefined]);
  const bands = holderBands(history, sources);
  assert.deepEqual(bands[0], [{ firstHour: 0, lastHour: 9, value: 0, breakBefore: true }, { firstHour: 10, lastHour: 19, value: 400, breakBefore: false },
    { firstHour: 40, lastHour: 49, value: 0, breakBefore: true }]);
  assert.deepEqual(bands[1], [{ firstHour: 0, lastHour: 19, value: 100, breakBefore: true }, { firstHour: 40, lastHour: 49, value: 0, breakBefore: true }]);
  assert.equal(sourceTagAt(history, 9, 12), null);
  assert.equal(sourceTagAt(history, 9, 16), 'ITA');
  assert.throws(() => validateReserveHistory({ ...history, segments: [history.segments[1]!, history.segments[0]!] }));
});
