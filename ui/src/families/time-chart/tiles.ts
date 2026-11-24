import {
  buildSummaryShape,
  type BucketStreamReader,
  type ChartBucket,
  type SummaryShape,
} from '../../charts/chartSummary.ts';
import type { MarkCluster, MarkSummary } from '../../charts/markSummary.ts';
import { countryKey } from '../../dashboard/countryKey.ts';
import {
  FIRST_BREAK,
  HAS_BREAK,
  LEVEL,
  PRESENT,
  RUN,
  SEAM,
  TILE_BINS,
  binHoursOf,
  bytesOf,
  type TileReply,
} from '../../dashboard/tiles/tile.ts';
import { cellClusters } from '../../dashboard/tiles/view.ts';
import type {
  HeadLine,
  HeadMarks,
  HeadOpenBin,
  HourSpan,
  MissingSpan,
  SeriesHeadReply,
  TilePart,
} from '../../generated/dashboard';

export { TILE_BINS, binHoursOf };

const I32_MIN = -2147483648;
const I32_MAX = 2147483647;
const ALL_FLAGS = PRESENT | FIRST_BREAK | HAS_BREAK | LEVEL | RUN | SEAM;
const NON_HOURLY_FLAGS = PRESENT | FIRST_BREAK | HAS_BREAK | LEVEL;
const HOURLY_FLAGS = PRESENT | FIRST_BREAK | RUN | SEAM;
const DEFAULT_RING_CAPACITY = 64;
const DECIMAL_RE = /^(0|[1-9]\d*)$/;

export type SeriesKey = string;

export function seriesKey(country: string, role = 'primary'): SeriesKey {
  return `${role}\0${country}`;
}

export function parseSeriesKey(key: SeriesKey): { readonly role: string; readonly country: string } {
  const sep = key.indexOf('\0');
  if (sep < 0) return { role: 'primary', country: key };
  return { role: key.slice(0, sep), country: key.slice(sep + 1) };
}

export interface TileRequestIdentity {
  readonly sourceId: string;
  readonly revision: string;
  readonly branchId: string;
  readonly kind: string;
  readonly measure: string;
  readonly role: string;
  readonly country: string;
  readonly binHours: number;
  readonly index: number;
  readonly withMarks: boolean;
  readonly within?: readonly HourSpan[];
}

export function tileIdentityKey(identity: TileRequestIdentity): string {
  const withinPart = identity.within
    ? `|w:${identity.within.map(span => `${span.firstHour}:${span.lastHour}`).join(',')}`
    : '';
  return [
    identity.sourceId,
    identity.revision,
    identity.branchId,
    identity.kind,
    identity.measure,
    identity.role,
    identity.country,
    String(identity.binHours),
    String(identity.index),
    identity.withMarks ? '1' : '0',
  ].join('|') + withinPart;
}

export interface IndexedDrop {
  readonly binStart: number;
  readonly count: number;
  readonly firstOrdinal: string;
}

export interface DecodedTile {
  readonly key: string;
  readonly identity: TileRequestIdentity;
  readonly seriesKey: SeriesKey;
  readonly binHours: number;
  readonly index: number;
  readonly firstHour: number;
  readonly lastHour: number;
  readonly immutable: boolean;
  readonly filled: number | null;
  readonly decimals: number;
  readonly present: Uint8Array;
  readonly flags: Uint8Array;
  readonly firstHours: Int32Array;
  readonly lastHours: Int32Array;
  readonly lastValueHours: Int32Array;
  readonly minHours: Int32Array;
  readonly maxHours: Int32Array;
  readonly firstValues: Float64Array;
  readonly lastValues: Float64Array;
  readonly minValues: Float64Array;
  readonly maxValues: Float64Array;
  readonly missing: readonly MissingSpan[];
  readonly parts: readonly TilePart[];
  readonly marksBefore: string | null;
  readonly drops: readonly IndexedDrop[];
  readonly bytes: number;
}

function isInt32(value: unknown): value is number {
  return typeof value === 'number' && Number.isInteger(value) && value >= I32_MIN && value <= I32_MAX;
}

function isPositiveBinHours(value: unknown): value is number {
  return typeof value === 'number' && Number.isInteger(value) && value >= 1 && value <= Math.floor(I32_MAX / TILE_BINS);
}

function malformed(detail: string): Error {
  return new Error(`The chart tile reply is malformed: ${detail}.`);
}

function decodeMarks(
  marks: { readonly before: string; readonly bins: readonly (readonly [number, number])[] } | undefined,
  firstHour: number,
  binHours: number,
): { readonly before: string | null; readonly drops: readonly IndexedDrop[] } {
  if (!marks) return { before: null, drops: [] };
  if (typeof marks.before !== 'string' || !DECIMAL_RE.test(marks.before) || !Array.isArray(marks.bins)) {
    throw malformed('invalid marks envelope');
  }
  let ordinal = BigInt(marks.before);
  let prevBin = -1;
  const drops: IndexedDrop[] = [];
  for (let i = 0; i < marks.bins.length; i++) {
    const entry = marks.bins[i];
    if (!Array.isArray(entry) || entry.length !== 2) throw malformed('invalid mark entry');
    const [bin, count] = entry;
    if (!Number.isInteger(bin) || bin < 0 || bin >= TILE_BINS || bin <= prevBin) {
      throw malformed('out-of-order mark bin');
    }
    if (!Number.isSafeInteger(count) || count <= 0) {
      throw malformed('invalid mark count');
    }
    prevBin = bin;
    const binStart = firstHour + bin * binHours;
    if (!isInt32(binStart)) throw malformed('mark hour out of i32 range');
    drops.push({ binStart, count, firstOrdinal: ordinal.toString() });
    ordinal += BigInt(count);
  }
  return { before: marks.before, drops };
}

export function decodeTilesStrict(
  reply: TileReply,
  identities: readonly TileRequestIdentity[],
): DecodedTile[] {
  if (!reply || typeof reply !== 'object' || !Array.isArray(reply.lines) || reply.lines.length !== identities.length) {
    throw malformed('line count mismatch');
  }
  if (!identities.length) return [];
  const expectedBinHours = identities[0]!.binHours;
  const expectedIndex = identities[0]!.index;
  if (!isPositiveBinHours(expectedBinHours) || !Number.isSafeInteger(expectedIndex)) {
    throw malformed('invalid requested tile identity');
  }
  for (let i = 1; i < identities.length; i++) {
    if (identities[i]!.binHours !== expectedBinHours || identities[i]!.index !== expectedIndex) {
      throw malformed('mixed tile identities in batch');
    }
  }
  if (reply.binHours !== expectedBinHours || reply.index !== expectedIndex) {
    throw malformed('tile coordinates mismatch');
  }
  const expectedFirstHour = expectedIndex * TILE_BINS * expectedBinHours;
  const expectedLastHour = expectedFirstHour + TILE_BINS * expectedBinHours - 1;
  if (!isInt32(reply.firstHour) || reply.firstHour !== expectedFirstHour || !isInt32(expectedLastHour)) {
    throw malformed('firstHour out of i32 bounds');
  }
  if (!Number.isInteger(reply.decimals) || reply.decimals < 0 || reply.decimals > 12) {
    throw malformed('invalid decimals');
  }
  if (reply.filled !== null && (!isInt32(reply.filled) || reply.filled < reply.firstHour || reply.filled > expectedLastHour)) {
    throw malformed('invalid filled hour');
  }
  if (!(typeof reply.bins === 'string' || reply.bins instanceof Uint8Array)) {
    throw malformed('missing binary payload');
  }
  const bytes = typeof reply.bins === 'string' ? bytesOf(reply.bins) : reply.bins;
  const lineCount = identities.length;
  const flagCount = lineCount * TILE_BINS;
  if (bytes.byteLength < flagCount) throw malformed('truncated flags');
  const rawFlags = bytes.subarray(0, flagCount);
  const scale = reply.rawValues ? 10 ** reply.decimals : 1;
  const valueBytes = reply.rawValues ? 4 : 8;
  const readValue = (view: DataView, byteOffset: number): number => {
    const v = reply.rawValues ? view.getInt32(byteOffset, true) / scale : view.getFloat64(byteOffset, true);
    if (!Number.isFinite(v)) throw malformed('non-finite value');
    return v;
  };
  const dataView = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);

  if (reply.hourBytes === 0) {
    if (expectedBinHours !== 1) throw malformed('hourly runs with binHours != 1');
    const runCounts = new Uint32Array(lineCount + 1);
    for (let li = 0; li < lineCount; li++) {
      let activeRun = false;
      for (let bin = 0; bin < TILE_BINS; bin++) {
        const flag = rawFlags[li * TILE_BINS + bin]!;
        if ((flag & ~HOURLY_FLAGS) !== 0) throw malformed('invalid hourly flag bits');
        if ((flag & PRESENT) === 0) {
          if (flag !== 0) throw malformed('non-zero flag on absent hourly slot');
          activeRun = false;
          continue;
        }
        if ((flag & RUN) !== 0) {
          if ((flag & SEAM) !== 0 && (flag & FIRST_BREAK) !== 0) {
            throw malformed('mutually exclusive SEAM and FIRST_BREAK');
          }
          runCounts[li + 1]!++;
          activeRun = true;
        } else {
          if ((flag & (SEAM | FIRST_BREAK)) !== 0) throw malformed('continuation slot carries break or seam flag');
          if (!activeRun) throw malformed('hourly continuation without active run');
        }
      }
      runCounts[li + 1]! += runCounts[li]!;
    }
    const totalRuns = runCounts[lineCount]!;
    const expectedBytes = flagCount + totalRuns * valueBytes;
    if (bytes.byteLength !== expectedBytes) throw malformed('hourly payload byte length mismatch');

    const out: DecodedTile[] = [];
    for (let li = 0; li < lineCount; li++) {
      const identity = identities[li]!;
      const line = reply.lines[li];
      if (!line || countryKey(line.country) !== identity.country || !Array.isArray(line.missing) || !Array.isArray(line.parts)) {
        throw malformed('tile line identity mismatch');
      }
      const { before, drops } = decodeMarks(line.marks, reply.firstHour, 1);
      const present = new Uint8Array(TILE_BINS);
      const flags = new Uint8Array(TILE_BINS);
      const firstHours = new Int32Array(TILE_BINS);
      const lastHours = new Int32Array(TILE_BINS);
      const lastValueHours = new Int32Array(TILE_BINS);
      const minHours = new Int32Array(TILE_BINS);
      const maxHours = new Int32Array(TILE_BINS);
      const firstValues = new Float64Array(TILE_BINS);
      const lastValues = new Float64Array(TILE_BINS);
      const minValues = new Float64Array(TILE_BINS);
      const maxValues = new Float64Array(TILE_BINS);

      let runAt = runCounts[li]!;
      let currentVal = 0;
      for (let bin = 0; bin < TILE_BINS; bin++) {
        const flag = rawFlags[li * TILE_BINS + bin]!;
        if ((flag & PRESENT) === 0) continue;
        const h = reply.firstHour + bin;
        let firstBreak = false;
        if ((flag & RUN) !== 0) {
          currentVal = readValue(dataView, flagCount + runAt * valueBytes);
          runAt++;
          firstBreak = (flag & FIRST_BREAK) !== 0;
        }
        present[bin] = 1;
        flags[bin] = firstBreak ? 1 : 0;
        firstHours[bin] = h;
        lastHours[bin] = h;
        lastValueHours[bin] = h;
        minHours[bin] = h;
        maxHours[bin] = h;
        firstValues[bin] = currentVal;
        lastValues[bin] = currentVal;
        minValues[bin] = currentVal;
        maxValues[bin] = currentVal;
      }

      out.push({
        key: tileIdentityKey(identity),
        identity,
        seriesKey: seriesKey(identity.country, identity.role),
        binHours: 1,
        index: expectedIndex,
        firstHour: reply.firstHour,
        lastHour: expectedLastHour,
        immutable: reply.immutable,
        filled: reply.filled,
        decimals: reply.decimals,
        present,
        flags,
        firstHours,
        lastHours,
        lastValueHours,
        minHours,
        maxHours,
        firstValues,
        lastValues,
        minValues,
        maxValues,
        missing: line.missing,
        parts: line.parts,
        marksBefore: before,
        drops,
        bytes: TILE_BINS * (1 + 1 + 5 * 4 + 4 * 8),
      });
    }
    return out;
  }

  const hourBytes = reply.hourBytes;
  if (hourBytes !== 1 && hourBytes !== 2 && hourBytes !== 4) {
    throw malformed('invalid hourBytes');
  }
  const presentCounts = new Uint32Array(lineCount + 1);
  const valueCounts = new Uint32Array(lineCount + 1);
  for (let li = 0; li < lineCount; li++) {
    for (let bin = 0; bin < TILE_BINS; bin++) {
      const flag = rawFlags[li * TILE_BINS + bin]!;
      if ((flag & ~ALL_FLAGS) !== 0 || (flag & ~NON_HOURLY_FLAGS) !== 0) {
        throw malformed('invalid binned flag bits');
      }
      if ((flag & PRESENT) === 0) {
        if (flag !== 0) throw malformed('non-zero flag on absent binned slot');
        continue;
      }
      presentCounts[li + 1]!++;
      valueCounts[li + 1]! += (flag & LEVEL) !== 0 ? 1 : 4;
    }
    presentCounts[li + 1]! += presentCounts[li]!;
    valueCounts[li + 1]! += valueCounts[li]!;
  }
  const totalPresent = presentCounts[lineCount]!;
  const totalValues = valueCounts[lineCount]!;
  const hoursStart = flagCount + totalValues * valueBytes;
  const expectedBytes = hoursStart + totalPresent * 5 * hourBytes;
  if (bytes.byteLength !== expectedBytes) {
    throw malformed('binned payload byte length mismatch');
  }

  const readHourOffset = (byteOffset: number): number => {
    if (hourBytes === 1) return dataView.getUint8(byteOffset);
    if (hourBytes === 2) return dataView.getUint16(byteOffset, true);
    return dataView.getUint32(byteOffset, true);
  };

  const out: DecodedTile[] = [];
  for (let li = 0; li < lineCount; li++) {
    const identity = identities[li]!;
    const line = reply.lines[li];
    if (!line || countryKey(line.country) !== identity.country || !Array.isArray(line.missing) || !Array.isArray(line.parts)) {
      throw malformed('tile line identity mismatch');
    }
    const { before, drops } = decodeMarks(line.marks, reply.firstHour, expectedBinHours);
    const present = new Uint8Array(TILE_BINS);
    const flags = new Uint8Array(TILE_BINS);
    const firstHours = new Int32Array(TILE_BINS);
    const lastHours = new Int32Array(TILE_BINS);
    const lastValueHours = new Int32Array(TILE_BINS);
    const minHours = new Int32Array(TILE_BINS);
    const maxHours = new Int32Array(TILE_BINS);
    const firstValues = new Float64Array(TILE_BINS);
    const lastValues = new Float64Array(TILE_BINS);
    const minValues = new Float64Array(TILE_BINS);
    const maxValues = new Float64Array(TILE_BINS);

    let pAt = presentCounts[li]!;
    let vAt = valueCounts[li]!;
    for (let bin = 0; bin < TILE_BINS; bin++) {
      const flag = rawFlags[li * TILE_BINS + bin]!;
      if ((flag & PRESENT) === 0) continue;
      const start = reply.firstHour + bin * expectedBinHours;
      const isLevel = (flag & LEVEL) !== 0;
      const vBase = flagCount + vAt * valueBytes;
      const fv = readValue(dataView, vBase);
      const lv = isLevel ? fv : readValue(dataView, vBase + valueBytes);
      const minV = isLevel ? fv : readValue(dataView, vBase + 2 * valueBytes);
      const maxV = isLevel ? fv : readValue(dataView, vBase + 3 * valueBytes);
      vAt += isLevel ? 1 : 4;

      const hBase = hoursStart + pAt * 5 * hourBytes;
      const fhOff = readHourOffset(hBase);
      const lhOff = readHourOffset(hBase + hourBytes);
      const lvhOff = readHourOffset(hBase + 2 * hourBytes);
      const minHOff = readHourOffset(hBase + 3 * hourBytes);
      const maxHOff = readHourOffset(hBase + 4 * hourBytes);
      pAt++;

      if (
        fhOff >= expectedBinHours
        || lhOff >= expectedBinHours
        || fhOff > lhOff
        || lvhOff < fhOff
        || lvhOff > lhOff
        || minHOff < fhOff
        || minHOff > lhOff
        || maxHOff < fhOff
        || maxHOff > lhOff
      ) {
        throw malformed('bucket hour offsets out of bin bounds');
      }

      present[bin] = 1;
      flags[bin] = ((flag & FIRST_BREAK) !== 0 ? 1 : 0) | ((flag & HAS_BREAK) !== 0 ? 2 : 0);
      firstHours[bin] = start + fhOff;
      lastHours[bin] = start + lhOff;
      lastValueHours[bin] = start + lvhOff;
      minHours[bin] = start + minHOff;
      maxHours[bin] = start + maxHOff;
      firstValues[bin] = fv;
      lastValues[bin] = lv;
      minValues[bin] = minV;
      maxValues[bin] = maxV;
    }

    out.push({
      key: tileIdentityKey(identity),
      identity,
      seriesKey: seriesKey(identity.country, identity.role),
      binHours: expectedBinHours,
      index: expectedIndex,
      firstHour: reply.firstHour,
      lastHour: expectedLastHour,
      immutable: reply.immutable,
      filled: reply.filled,
      decimals: reply.decimals,
      present,
      flags,
      firstHours,
      lastHours,
      lastValueHours,
      minHours,
      maxHours,
      firstValues,
      lastValues,
      minValues,
      maxValues,
      missing: line.missing,
      parts: line.parts,
      marksBefore: before,
      drops,
      bytes: TILE_BINS * (1 + 1 + 5 * 4 + 4 * 8),
    });
  }
  return out;
}

export class ClosedBinRing {
  readonly binHours: number;
  readonly capacity: number;
  private readonly binStarts: Float64Array;
  private readonly present: Uint8Array;
  private readonly flags: Uint8Array;
  private readonly firstHours: Int32Array;
  private readonly lastHours: Int32Array;
  private readonly lastValueHours: Int32Array;
  private readonly minHours: Int32Array;
  private readonly maxHours: Int32Array;
  private readonly firstValues: Float64Array;
  private readonly lastValues: Float64Array;
  private readonly minValues: Float64Array;
  private readonly maxValues: Float64Array;
  private readonly markCounts: Float64Array;
  private readonly markOrdinals: (string | null)[];
  private head = 0;
  private count = 0;
  private rev = 0;

  constructor(binHours: number, capacity = DEFAULT_RING_CAPACITY) {
    if (!isPositiveBinHours(binHours)) throw malformed('invalid ring binHours');
    this.binHours = binHours;
    this.capacity = Math.max(1, capacity);
    this.binStarts = new Float64Array(this.capacity);
    this.present = new Uint8Array(this.capacity);
    this.flags = new Uint8Array(this.capacity);
    this.firstHours = new Int32Array(this.capacity);
    this.lastHours = new Int32Array(this.capacity);
    this.lastValueHours = new Int32Array(this.capacity);
    this.minHours = new Int32Array(this.capacity);
    this.maxHours = new Int32Array(this.capacity);
    this.firstValues = new Float64Array(this.capacity);
    this.lastValues = new Float64Array(this.capacity);
    this.minValues = new Float64Array(this.capacity);
    this.maxValues = new Float64Array(this.capacity);
    this.markCounts = new Float64Array(this.capacity);
    this.markOrdinals = new Array<string | null>(this.capacity).fill(null);
  }

  get revision(): number {
    return this.rev;
  }

  get size(): number {
    return this.count;
  }

  clear(): void {
    if (this.count === 0) return;
    this.head = 0;
    this.count = 0;
    this.rev++;
  }

  private slotAt(logicalIndex: number): number {
    return (this.head + logicalIndex) % this.capacity;
  }

  private findSlot(binStart: number): number {
    let lo = 0;
    let hi = this.count;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      const start = this.binStarts[this.slotAt(mid)]!;
      if (start < binStart) lo = mid + 1;
      else hi = mid;
    }
    if (lo < this.count) {
      const slot = this.slotAt(lo);
      if (this.binStarts[slot] === binStart) return slot;
    }
    return -1;
  }

  internalPromote(binStart: number, bin: HeadOpenBin | null, marks?: HeadMarks | null): boolean {
    if (!isInt32(binStart) || Math.floor(binStart / this.binHours) * this.binHours !== binStart) {
      return false;
    }
    if (bin !== null) {
      if (
        !isInt32(bin.firstHour) || !isInt32(bin.lastHour) || !isInt32(bin.lastValueHour)
        || !isInt32(bin.minHour) || !isInt32(bin.maxHour)
        || bin.firstHour < binStart || bin.lastHour >= binStart + this.binHours || bin.firstHour > bin.lastHour
        || !Number.isFinite(bin.firstValue) || !Number.isFinite(bin.lastValue)
        || !Number.isFinite(bin.minValue) || !Number.isFinite(bin.maxValue)
      ) {
        return false;
      }
    }
    let target: number;
    if (this.count > 0) {
      const newestSlot = this.slotAt(this.count - 1);
      const newestStart = this.binStarts[newestSlot]!;
      if (binStart < newestStart) {
        const existing = this.findSlot(binStart);
        if (existing < 0) return false;
        target = existing;
      } else if (binStart === newestStart) {
        target = newestSlot;
      } else if (this.count < this.capacity) {
        target = this.slotAt(this.count);
        this.count++;
      } else {
        target = this.head;
        this.head = (this.head + 1) % this.capacity;
      }
    } else {
      target = 0;
      this.head = 0;
      this.count = 1;
    }

    this.binStarts[target] = binStart;
    if (bin) {
      this.present[target] = 1;
      this.flags[target] = (bin.firstBreak ? 1 : 0) | (bin.hasBreak ? 2 : 0);
      this.firstHours[target] = bin.firstHour;
      this.lastHours[target] = bin.lastHour;
      this.lastValueHours[target] = bin.lastValueHour;
      this.minHours[target] = bin.minHour;
      this.maxHours[target] = bin.maxHour;
      this.firstValues[target] = bin.firstValue;
      this.lastValues[target] = bin.lastValue;
      this.minValues[target] = bin.minValue;
      this.maxValues[target] = bin.maxValue;
    } else {
      this.present[target] = 0;
      this.flags[target] = 0;
    }
    if (marks && marks.count > 0 && DECIMAL_RE.test(marks.before)) {
      this.markCounts[target] = marks.count;
      this.markOrdinals[target] = marks.before;
    } else {
      this.markCounts[target] = 0;
      this.markOrdinals[target] = null;
    }
    this.rev++;
    return true;
  }

  forEachInRange(
    fromBinStart: number,
    toBinStart: number,
    visit: (
      binStart: number,
      present: boolean,
      bucket: ChartBucket | null,
      drop: IndexedDrop | null,
    ) => void,
  ): void {
    for (let i = 0; i < this.count; i++) {
      const slot = this.slotAt(i);
      const start = this.binStarts[slot]!;
      if (start < fromBinStart) continue;
      if (start > toBinStart) break;
      const isPresent = this.present[slot] !== 0;
      const bucket: ChartBucket | null = isPresent
        ? {
            firstHour: this.firstHours[slot]!,
            lastHour: this.lastHours[slot]!,
            lastValueHour: this.lastValueHours[slot]!,
            minHour: this.minHours[slot]!,
            maxHour: this.maxHours[slot]!,
            firstValue: this.firstValues[slot]!,
            lastValue: this.lastValues[slot]!,
            minValue: this.minValues[slot]!,
            maxValue: this.maxValues[slot]!,
            firstBreak: (this.flags[slot]! & 1) !== 0,
            hasBreak: (this.flags[slot]! & 2) !== 0,
          }
        : null;
      const count = this.markCounts[slot]!;
      const ord = this.markOrdinals[slot];
      const drop: IndexedDrop | null = count > 0 && ord !== null ? { binStart: start, count, firstOrdinal: ord } : null;
      visit(start, isPresent, bucket, drop);
    }
  }
}

export function promoteClosedBin(
  ring: ClosedBinRing,
  binStart: number,
  bin: HeadOpenBin | null,
  marks?: HeadMarks | null,
): boolean {
  return ring.internalPromote(binStart, bin, marks);
}

export interface RetainedClosedSeries {
  readonly seriesKey: SeriesKey;
  readonly windowKey: string;
  readonly revision: number;
  readonly binHours: number;
  readonly firstBinHour: number;
  readonly lastBinHour: number;
  readonly binCount: number;
  readonly held: boolean;
  readonly present: Uint8Array;
  readonly flags: Uint8Array;
  readonly firstHours: Int32Array;
  readonly lastHours: Int32Array;
  readonly lastValueHours: Int32Array;
  readonly minHours: Int32Array;
  readonly maxHours: Int32Array;
  readonly firstValues: Float64Array;
  readonly lastValues: Float64Array;
  readonly minValues: Float64Array;
  readonly maxValues: Float64Array;
  readonly stream: BucketStreamReader;
  readonly shape: SummaryShape;
  readonly missing: readonly MissingSpan[];
  readonly parts: readonly TilePart[];
  readonly drops: readonly IndexedDrop[];
  readonly decimals: number;
}

export function makeTypedStream(
  count: number,
  present: Uint8Array,
  flags: Uint8Array,
  firstHours: Int32Array,
  lastHours: Int32Array,
  lastValueHours: Int32Array,
  minHours: Int32Array,
  maxHours: Int32Array,
  firstValues: Float64Array,
  lastValues: Float64Array,
  minValues: Float64Array,
  maxValues: Float64Array,
): BucketStreamReader {
  return {
    count,
    present: i => present[i] !== 0,
    firstHour: i => firstHours[i]!,
    lastHour: i => lastHours[i]!,
    firstValue: i => firstValues[i]!,
    lastValue: i => lastValues[i]!,
    lastValueHour: i => lastValueHours[i]!,
    minHour: i => minHours[i]!,
    minValue: i => minValues[i]!,
    maxHour: i => maxHours[i]!,
    maxValue: i => maxValues[i]!,
    firstBreak: i => (flags[i]! & 1) !== 0,
    hasBreak: i => (flags[i]! & 2) !== 0,
  };
}

export class RetainedClosedWindowStore {
  private readonly retained = new Map<SeriesKey, RetainedClosedSeries>();
  private nextRevision = 1;

  retain(
    key: SeriesKey,
    tiles: readonly DecodedTile[],
    firstIndex: number,
    lastIndex: number,
    binHours: number,
    held: boolean,
    ring?: ClosedBinRing,
  ): RetainedClosedSeries | null {
    if (tiles.length === 0 || lastIndex < firstIndex) return null;
    const tileIds = tiles.map(t => `${t.key}@${t.filled ?? 'null'}:${t.immutable ? 1 : 0}`).join(';');
    const ringRev = ring && ring.binHours === binHours ? ring.revision : 0;
    const windowKey = `${binHours}|${firstIndex}|${lastIndex}|${held ? 1 : 0}|${ringRev}|${tileIds}`;
    const existing = this.retained.get(key);
    if (existing && existing.windowKey === windowKey) {
      return existing;
    }

    const tileCount = lastIndex - firstIndex + 1;
    const binCount = tileCount * TILE_BINS;
    const firstBinHour = firstIndex * TILE_BINS * binHours;
    const lastBinHour = (lastIndex + 1) * TILE_BINS * binHours - binHours;

    const present = new Uint8Array(binCount);
    const flags = new Uint8Array(binCount);
    const firstHours = new Int32Array(binCount);
    const lastHours = new Int32Array(binCount);
    const lastValueHours = new Int32Array(binCount);
    const minHours = new Int32Array(binCount);
    const maxHours = new Int32Array(binCount);
    const firstValues = new Float64Array(binCount);
    const lastValues = new Float64Array(binCount);
    const minValues = new Float64Array(binCount);
    const maxValues = new Float64Array(binCount);

    const missing: MissingSpan[] = [];
    const parts: TilePart[] = [];
    const seenParts = new Set<string>();
    const dropsByBin = new Map<number, IndexedDrop>();
    let decimals = tiles[0]!.decimals;

    for (let ti = 0; ti < tiles.length; ti++) {
      const tile = tiles[ti]!;
      decimals = tile.decimals;
      const offset = (tile.index - firstIndex) * TILE_BINS;
      if (offset < 0 || offset + TILE_BINS > binCount) continue;
      present.set(tile.present, offset);
      flags.set(tile.flags, offset);
      firstHours.set(tile.firstHours, offset);
      lastHours.set(tile.lastHours, offset);
      lastValueHours.set(tile.lastValueHours, offset);
      minHours.set(tile.minHours, offset);
      maxHours.set(tile.maxHours, offset);
      firstValues.set(tile.firstValues, offset);
      lastValues.set(tile.lastValues, offset);
      minValues.set(tile.minValues, offset);
      maxValues.set(tile.maxValues, offset);

      for (const span of tile.missing) {
        const prev = missing.at(-1);
        if (prev && prev.reason === span.reason && prev.lastHour + 1 === span.firstHour) {
          missing[missing.length - 1] = { ...prev, lastHour: span.lastHour };
        } else {
          missing.push({ ...span });
        }
      }
      for (const part of tile.parts) {
        const pk = `${part.firstHour}:${part.lastHour}:${part.slot}:${part.countryId}`;
        if (!seenParts.has(pk)) {
          seenParts.add(pk);
          parts.push(part);
        }
      }
      for (const drop of tile.drops) {
        dropsByBin.set(drop.binStart, drop);
      }
    }

    if (ring && ring.binHours === binHours && ring.size > 0) {
      ring.forEachInRange(firstBinHour, lastBinHour, (binStart, isPresent, bucket, drop) => {
        const slot = Math.floor((binStart - firstBinHour) / binHours);
        if (slot < 0 || slot >= binCount) return;
        if (isPresent && bucket) {
          present[slot] = 1;
          flags[slot] = (bucket.firstBreak ? 1 : 0) | (bucket.hasBreak ? 2 : 0);
          firstHours[slot] = bucket.firstHour;
          lastHours[slot] = bucket.lastHour;
          lastValueHours[slot] = bucket.lastValueHour;
          minHours[slot] = bucket.minHour;
          maxHours[slot] = bucket.maxHour;
          firstValues[slot] = bucket.firstValue;
          lastValues[slot] = bucket.lastValue;
          minValues[slot] = bucket.minValue;
          maxValues[slot] = bucket.maxValue;
        }
        if (drop) {
          dropsByBin.set(binStart, drop);
        }
      });
    }

    const drops = [...dropsByBin.values()].sort((a, b) => a.binStart - b.binStart);
    const stream = makeTypedStream(
      binCount,
      present,
      flags,
      firstHours,
      lastHours,
      lastValueHours,
      minHours,
      maxHours,
      firstValues,
      lastValues,
      minValues,
      maxValues,
    );
    const shape = buildSummaryShape(stream, held, Infinity);
    const built: RetainedClosedSeries = {
      seriesKey: key,
      windowKey,
      revision: this.nextRevision++,
      binHours,
      firstBinHour,
      lastBinHour,
      binCount,
      held,
      present,
      flags,
      firstHours,
      lastHours,
      lastValueHours,
      minHours,
      maxHours,
      firstValues,
      lastValues,
      minValues,
      maxValues,
      stream,
      shape,
      missing,
      parts,
      drops,
      decimals,
    };
    this.retained.set(key, built);
    return built;
  }

  get(key: SeriesKey): RetainedClosedSeries | undefined {
    return this.retained.get(key);
  }

  delete(key: SeriesKey): void {
    this.retained.delete(key);
  }

  clear(): void {
    this.retained.clear();
  }
}

export function closedBucketAt(closed: RetainedClosedSeries, binStart: number): ChartBucket | null {
  const slot = Math.floor((binStart - closed.firstBinHour) / closed.binHours);
  if (slot < 0 || slot >= closed.binCount || closed.present[slot] === 0) return null;
  return {
    firstHour: closed.firstHours[slot]!,
    lastHour: closed.lastHours[slot]!,
    lastValueHour: closed.lastValueHours[slot]!,
    minHour: closed.minHours[slot]!,
    maxHour: closed.maxHours[slot]!,
    firstValue: closed.firstValues[slot]!,
    lastValue: closed.lastValues[slot]!,
    minValue: closed.minValues[slot]!,
    maxValue: closed.maxValues[slot]!,
    firstBreak: (closed.flags[slot]! & 1) !== 0,
    hasBreak: (closed.flags[slot]! & 2) !== 0,
  };
}

export function closedExtent(
  closed: RetainedClosedSeries,
  from: number,
  to: number,
  cutBefore = Infinity,
): [number, number] | null {
  const startBin = Math.max(0, Math.floor((from - closed.firstBinHour) / closed.binHours));
  const endBin = Math.min(closed.binCount - 1, Math.floor((to - closed.firstBinHour) / closed.binHours));
  let low = Infinity;
  let high = -Infinity;
  for (let i = startBin; i <= endBin; i++) {
    if (closed.present[i] === 0) continue;
    const lh = closed.lastHours[i]!;
    if (lh >= cutBefore) break;
    if (lh < from) continue;
    if (closed.firstHours[i]! > to) break;
    const minV = closed.minValues[i]!;
    const maxV = closed.maxValues[i]!;
    if (minV < low) low = minV;
    if (maxV > high) high = maxV;
  }
  return low <= high ? [low, high] : null;
}

export function closedEndValue(
  closed: RetainedClosedSeries,
  atHour: number,
  cutBefore = Infinity,
): number | null {
  const maxSlot = Math.min(closed.binCount - 1, Math.floor((Math.min(atHour, cutBefore - 1) - closed.firstBinHour) / closed.binHours));
  for (let i = maxSlot; i >= 0; i--) {
    if (closed.present[i] === 0) continue;
    if (closed.firstHours[i]! > atHour) continue;
    if (closed.lastHours[i]! >= cutBefore) continue;
    return closed.lastValues[i]!;
  }
  return null;
}

export function headOpenBinAsBucket(open: HeadOpenBin): ChartBucket {
  return {
    firstHour: open.firstHour,
    lastHour: open.lastHour,
    firstValue: open.firstValue,
    lastValue: open.lastValue,
    lastValueHour: open.lastValueHour,
    minHour: open.minHour,
    minValue: open.minValue,
    maxHour: open.maxHour,
    maxValue: open.maxValue,
    firstBreak: open.firstBreak,
    hasBreak: open.hasBreak,
  };
}

export function composeMarkSummary(
  closed: RetainedClosedSeries | null,
  head: SeriesHeadReply | null,
  headLine: HeadLine | null,
  from: number,
  to: number,
  width: number,
  cell: number,
  until: number | null,
  color: string,
  tag: string,
): MarkSummary | null {
  const binHours = closed?.binHours ?? head?.binHours ?? 0;
  if (binHours <= 0) return null;
  const cut = until !== null && head ? head.binFirstHour : Infinity;
  const rawDrops: [number, number, bigint][] = [];
  if (closed) {
    for (let i = 0; i < closed.drops.length; i++) {
      const drop = closed.drops[i]!;
      if (drop.binStart >= cut) break;
      if (until !== null && drop.binStart > until) break;
      rawDrops.push([drop.binStart, drop.count, BigInt(drop.firstOrdinal)]);
    }
  }
  if (head && headLine?.marks && headLine.marks.count > 0) {
    const binStart = head.binFirstHour;
    if (until === null || binStart <= until) {
      if (rawDrops.length === 0 || rawDrops[rawDrops.length - 1]![0] < binStart) {
        rawDrops.push([binStart, headLine.marks.count, BigInt(headLine.marks.before)]);
      }
    }
  }
  const clusters: MarkCluster[] = cellClusters(rawDrops, binHours, cell).map(cluster => ({ ...cluster, tag }));
  if (head && headLine?.marks && headLine.marks.count > 0 && (until === null || head.hour <= until)) {
    const last = clusters.at(-1);
    if (last && last.firstHour <= head.binFirstHour && last.lastHour > head.hour) {
      const clampedBins = last.bins?.map(([binLast, count]) => [Math.min(binLast, head.hour), count] as readonly [number, number]);
      clusters[clusters.length - 1] = { ...last, lastHour: head.hour, ...(clampedBins ? { bins: clampedBins } : {}) };
    }
  }
  return { from, to, width, clusters, placement: 'ticks', color, tiles: true };
}
