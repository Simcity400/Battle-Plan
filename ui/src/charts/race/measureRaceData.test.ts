import assert from 'node:assert/strict';
import test from 'node:test';
import { formatRaw, validateRankReply } from './measureRaceData.ts';
import type { SeriesRankReply, SeriesState } from '../../generated/dashboard';

const state = (tag: string, valueRaw: string | null, missing: string | null = null): SeriesState => ({ country: tag, hour: 50, drop: null, missing,
  run: valueRaw === null ? null : { firstHour: 40, lastHour: 60, valueRaw, slot: 1, countryId: 1, startsSegment: false, segmentFirstHour: 0, firstExact: true, lastExact: true } });

const rank = (states: readonly SeriesState[], decimals: number, from = 50, until = 50, refused: string | null = null): SeriesRankReply => ({
  branchId: '1', hour: 50, from, until, refused, measures: [{ name: 'free7', decimals }], rows: states.map(held => ({ country: held.country, slot: null, readings: [held] })),
});

test('a fixed-point raw value is written exactly, grouped, with at most the digits asked', () => {
  assert.equal(formatRaw('123456789', 5), '1,234');
  assert.equal(formatRaw('123456789', 5, 2), '1,234.56');
  assert.equal(formatRaw('-5', 5, 2), '0', 'a value showing as zero has no sign');
  assert.equal(formatRaw('-150000', 5, 1), '−1.5');
  assert.equal(formatRaw('42', 0, 3), '42');
});

test('rank replies that are malformed are refused', () => {
  assert.equal(validateRankReply(rank([state('GER', '4')], 0)).rows.length, 1);
  assert.throws(() => validateRankReply({ decimals: 0, states: [{ tag: 'GER', hour: 3, run: { valueRaw: 4 } }] }));
  assert.throws(() => validateRankReply({ ...rank([state('GER', '4')], 0), from: 51 }), 'a reply must hold its own hour');
});
