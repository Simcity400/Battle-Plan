import assert from 'node:assert/strict';
import test from 'node:test';
import {
  FIRST_BREAK,
  HAS_BREAK,
  LEVEL,
  PRESENT,
  RUN,
  SEAM,
  TILE_BINS,
  type TileReply,
} from '../../dashboard/tiles/tile.ts';
import type { HeadOpenBin, SeriesHeadReply } from '../../generated/dashboard';
import {
  ClosedBinRing,
  RetainedClosedWindowStore,
  closedBucketAt,
  closedEndValue,
  closedExtent,
  composeMarkSummary,
  decodeTilesStrict,
  parseSeriesKey,
  promoteClosedBin,
  seriesKey,
  tileIdentityKey,
  type TileRequestIdentity,
} from './tiles.ts';

function identity(overrides: Partial<TileRequestIdentity> = {}): TileRequestIdentity {
  return {
    sourceId: 'src-1',
    revision: 'rev-1',
    branchId: '1',
    kind: 'country-measure',
    measure: 'politicalPower',
    role: 'primary',
    country: 'GER',
    binHours: 6,
    index: 0,
    withMarks: true,
    ...overrides,
  };
}

function makeBinnedPayload(
  bins: readonly {
    readonly slot: number;
    readonly firstOff: number;
    readonly lastOff: number;
    readonly lastValOff: number;
    readonly minOff: number;
    readonly maxOff: number;
    readonly firstVal: number;
    readonly lastVal: number;
    readonly minVal: number;
    readonly maxVal: number;
    readonly firstBreak?: boolean;
    readonly hasBreak?: boolean;
    readonly level?: boolean;
  }[],
  scale = 100_000,
): Uint8Array {
  const flags = new Uint8Array(TILE_BINS);
  let valueCount = 0;
  for (const b of bins) {
    flags[b.slot] =
      PRESENT
      | (b.firstBreak ? FIRST_BREAK : 0)
      | (b.hasBreak ? HAS_BREAK : 0)
      | (b.level ? LEVEL : 0);
    valueCount += b.level ? 1 : 4;
  }
  const valueBytes = 4;
  const hourBytes = 1;
  const hoursStart = TILE_BINS + valueCount * valueBytes;
  const totalBytes = hoursStart + bins.length * 5 * hourBytes;
  const bytes = new Uint8Array(totalBytes);
  bytes.set(flags, 0);
  const view = new DataView(bytes.buffer);
  let vAt = TILE_BINS;
  let hAt = hoursStart;
  for (const b of bins) {
    if (b.level) {
      view.setInt32(vAt, Math.round(b.firstVal * scale), true);
      vAt += 4;
    } else {
      view.setInt32(vAt, Math.round(b.firstVal * scale), true);
      view.setInt32(vAt + 4, Math.round(b.lastVal * scale), true);
      view.setInt32(vAt + 8, Math.round(b.minVal * scale), true);
      view.setInt32(vAt + 12, Math.round(b.maxVal * scale), true);
      vAt += 16;
    }
    view.setUint8(hAt, b.firstOff);
    view.setUint8(hAt + 1, b.lastOff);
    view.setUint8(hAt + 2, b.lastValOff);
    view.setUint8(hAt + 3, b.minOff);
    view.setUint8(hAt + 4, b.maxOff);
    hAt += 5;
  }
  return bytes;
}

test('seriesKey separates roles without colliding on same country', () => {
  const a = seriesKey('GER', 'primary');
  const b = seriesKey('GER', 'ceiling');
  assert.notEqual(a, b);
  assert.deepEqual(parseSeriesKey(a), { role: 'primary', country: 'GER' });
  assert.deepEqual(parseSeriesKey(b), { role: 'ceiling', country: 'GER' });
});

test('decodeTilesStrict decodes every bin field and pre-indexes mark ordinals', () => {
  const id = identity({ binHours: 6, index: 0 });
  const payload = makeBinnedPayload([
    {
      slot: 0,
      firstOff: 0,
      lastOff: 5,
      lastValOff: 4,
      minOff: 2,
      maxOff: 3,
      firstVal: 10,
      lastVal: 25,
      minVal: 5,
      maxVal: 40,
      firstBreak: true,
      hasBreak: false,
    },
    {
      slot: 1,
      firstOff: 0,
      lastOff: 5,
      lastValOff: 0,
      minOff: 0,
      maxOff: 0,
      firstVal: 25,
      lastVal: 25,
      minVal: 25,
      maxVal: 25,
      level: true,
      hasBreak: true,
    },
  ]);
  const reply: TileReply = {
    branchId: '1',
    measure: 'politicalPower',
    decimals: 5,
    binHours: 6,
    index: 0,
    firstHour: 0,
    immutable: true,
    filled: 11,
    rawValues: true,
    hourBytes: 1,
    lines: [{
      country: 'GER',
      missing: [],
      parts: [{ firstHour: 0, lastHour: 11, slot: 7, countryId: 9001, segmentFirstHour: 0 }],
      marks: { before: '10', bins: [[0, 2], [1, 3]] },
    }],
    bins: payload,
  };
  const decoded = decodeTilesStrict(reply, [id]);
  assert.equal(decoded.length, 1);
  const tile = decoded[0]!;
  assert.equal(tile.key, tileIdentityKey(id));
  assert.equal(tile.firstHours[0], 0);
  assert.equal(tile.lastHours[0], 5);
  assert.equal(tile.lastValueHours[0], 4);
  assert.equal(tile.minHours[0], 2);
  assert.equal(tile.maxHours[0], 3);
  assert.equal(tile.firstValues[0], 10);
  assert.equal(tile.lastValues[0], 25);
  assert.equal(tile.minValues[0], 5);
  assert.equal(tile.maxValues[0], 40);
  assert.equal(tile.flags[0], 1);
  assert.equal(tile.firstValues[1], 25);
  assert.equal(tile.lastValues[1], 25);
  assert.equal(tile.minValues[1], 25);
  assert.equal(tile.maxValues[1], 25);
  assert.equal(tile.flags[1], 2);
  assert.deepEqual(tile.drops, [
    { binStart: 0, count: 2, firstOrdinal: '10' },
    { binStart: 6, count: 3, firstOrdinal: '12' },
  ]);
});

test('decodeTilesStrict rejects out-of-order marks, bad flags, and out-of-range hours', () => {
  const id = identity({ binHours: 6, index: 0 });
  const payload = makeBinnedPayload([{
    slot: 0, firstOff: 0, lastOff: 5, lastValOff: 0, minOff: 0, maxOff: 0,
    firstVal: 1, lastVal: 1, minVal: 1, maxVal: 1, level: true,
  }]);
  const baseReply: TileReply = {
    branchId: '1', measure: 'politicalPower', decimals: 5, binHours: 6, index: 0, firstHour: 0,
    immutable: true, filled: 5, rawValues: true, hourBytes: 1,
    lines: [{ country: 'GER', missing: [], parts: [], marks: { before: '0', bins: [[1, 1], [1, 1]] } }],
    bins: payload,
  };
  assert.throws(() => decodeTilesStrict(baseReply, [id]), /out-of-order mark bin/);

  const badFlags = payload.slice();
  badFlags[0] = PRESENT | RUN;
  assert.throws(() => decodeTilesStrict({ ...baseReply, lines: [{ country: 'GER', missing: [], parts: [] }], bins: badFlags }, [id]), /invalid binned flag bits/);

  const hourlyId = identity({ binHours: 1, index: 0 });
  const badHourly = new Uint8Array(TILE_BINS + 4);
  badHourly[0] = PRESENT | RUN | SEAM | FIRST_BREAK;
  assert.throws(() => decodeTilesStrict({
    ...baseReply, binHours: 1, hourBytes: 0,
    lines: [{ country: 'GER', missing: [], parts: [] }],
    bins: badHourly,
  }, [hourlyId]), /mutually exclusive SEAM and FIRST_BREAK/);
});

test('RetainedClosedWindowStore retains arrays and SummaryShape by reference across intra-bin updates and promotes closed bins via ClosedBinRing', () => {
  const id = identity({ binHours: 6, index: 0 });
  const payload = makeBinnedPayload([{
    slot: 0, firstOff: 0, lastOff: 5, lastValOff: 5, minOff: 0, maxOff: 5,
    firstVal: 10, lastVal: 20, minVal: 10, maxVal: 20, firstBreak: true,
  }]);
  const [tile] = decodeTilesStrict({
    branchId: '1', measure: 'politicalPower', decimals: 5, binHours: 6, index: 0, firstHour: 0,
    immutable: false, filled: 5, rawValues: true, hourBytes: 1,
    lines: [{ country: 'GER', missing: [], parts: [], marks: { before: '0', bins: [[0, 1]] } }],
    bins: payload,
  }, [id]);

  const store = new RetainedClosedWindowStore();
  const ring = new ClosedBinRing(6, 4);
  const key = seriesKey('GER');
  const first = store.retain(key, [tile], 0, 0, 6, false, ring)!;
  const second = store.retain(key, [tile], 0, 0, 6, false, ring)!;
  assert.strictEqual(first, second);
  assert.strictEqual(first.shape, second.shape);
  assert.strictEqual(first.firstValues, second.firstValues);

  const promoted: HeadOpenBin = {
    firstHour: 6, lastHour: 11, firstValue: 20, lastValue: 35, lastValueHour: 11,
    minHour: 6, minValue: 20, maxHour: 10, maxValue: 40, firstBreak: false, hasBreak: false,
  };
  assert.equal(promoteClosedBin(ring, 6, promoted, { before: '1', count: 2 }), true);
  const third = store.retain(key, [tile], 0, 0, 6, false, ring)!;
  assert.notStrictEqual(third, first);
  assert.deepEqual(closedBucketAt(third, 6), {
    firstHour: 6, lastHour: 11, firstValue: 20, lastValue: 35, lastValueHour: 11,
    minHour: 6, minValue: 20, maxHour: 10, maxValue: 40, firstBreak: false, hasBreak: false,
  });
  assert.deepEqual(closedExtent(third, 0, 11), [10, 40]);
  assert.equal(closedEndValue(third, 11), 35);

  const headReply: SeriesHeadReply = {
    branchId: '1', measure: 'politicalPower', decimals: 5, hour: 14, binHours: 6,
    binFirstHour: 12, binLastHour: 17, closedThrough: 11,
    lines: [{
      country: 'GER',
      openBin: { firstHour: 12, lastHour: 14, firstValue: 35, lastValue: 50, lastValueHour: 14, minHour: 12, minValue: 35, maxHour: 14, maxValue: 50, firstBreak: false, hasBreak: false },
      value: { valueRaw: '5000000', value: 50 },
      missing: [],
      marks: { before: '3', count: 1 },
    }],
  };
  const marks = composeMarkSummary(third, headReply, headReply.lines[0]!, 0, 17, 200, 1, 14, '#fff', 'GER')!;
  assert.deepEqual(marks.clusters.map(c => [c.firstHour, c.lastHour, c.firstOrdinal, c.count]), [
    [0, 5, '0', '1'],
    [6, 11, '1', '2'],
    [12, 14, '3', '1'],
  ]);
});
