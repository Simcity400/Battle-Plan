import type { ChartBucket } from '../charts/chartSummary';
import type { LiveValues } from '../charts/live';
import type { MarkCluster } from '../charts/markSummary';
import type { Run } from '../charts/series';
import type { FollowedAnswer } from '../dashboard/follow/message';
import type { TiledLine } from '../dashboard/tiles/view';
import type { MissingSpan } from '../generated/dashboard';
import type {
  ArtDemand,
  ArtHandle,
  ArtProvider,
  ArtResult,
  BoundedHover,
  BoundedMarkCluster,
  BoundedReadView,
  BoundedSeries,
  BoundedSlice,
  DisplayedHour,
  EntityKind,
  IdentityDemand,
  IdentityProvider,
  IdentityResult,
} from './plan.ts';
import type { DatedLogRow, LogRowSlot, TableRowSource } from './table/rowSource.ts';
import type { Value } from './value.ts';

export class FutureDataError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'FutureDataError';
  }
}

export function displayedHour(hour: number): DisplayedHour {
  if (!Number.isFinite(hour)) {
    throw new FutureDataError(`Invalid displayed hour: ${String(hour)}`);
  }
  return Math.floor(hour) as DisplayedHour;
}

export interface WitnessedPoint {
  readonly hour: number;
  readonly value: number;
  readonly breakBefore?: boolean;
}

export interface StraddlingCutResult {
  readonly bucket: ChartBucket | null;
  readonly straddling: 'none' | 'cut' | 'loading';
}

export interface BoundedWorkCounter {
  visits: number;
  allocations: number;
}

export function createBoundedSlice<T>(
  source: readonly T[],
  closedLength: number,
  tail?: T,
): BoundedSlice<T> {
  const hasTail = tail !== undefined;
  const length = closedLength + (hasTail ? 1 : 0);
  return {
    closedLength,
    length,
    ...(hasTail ? { tail } : {}),
    at(index: number): T | undefined {
      if (index < 0 || index >= length) return undefined;
      if (index < closedLength) return source[index];
      return tail;
    },
    *[Symbol.iterator](): Iterator<T> {
      for (let i = 0; i < closedLength; i++) yield source[i]!;
      if (hasTail) yield tail!;
    },
  };
}

const EMPTY_SLICE_CACHE = createBoundedSlice<never>([], 0);

export function emptySlice<T>(): BoundedSlice<T> {
  return EMPTY_SLICE_CACHE as unknown as BoundedSlice<T>;
}

function upperBoundFirst<T extends { readonly firstHour: number }>(
  items: readonly T[],
  hour: number,
  counter?: BoundedWorkCounter,
): number {
  let lo = 0;
  let hi = items.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (counter) counter.visits++;
    if (items[mid]!.firstHour <= hour) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

function lowerBoundRunOverlap(
  runs: readonly Run[],
  bucketStart: number,
  counter?: BoundedWorkCounter,
): number {
  let lo = 0;
  let hi = runs.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (counter) counter.visits++;
    if (runs[mid]!.lastHour < bucketStart) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

function lowerBoundPoint(
  points: readonly WitnessedPoint[],
  bucketStart: number,
  counter?: BoundedWorkCounter,
): number {
  let lo = 0;
  let hi = points.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (counter) counter.visits++;
    if (points[mid]!.hour < bucketStart) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

export interface StraddleResume {
  state: StraddleState | null;
}

interface StraddleState {
  readonly coarse: ChartBucket;
  readonly runs: readonly Run[] | null;
  readonly points: readonly WitnessedPoint[] | null;
  hour: number;
  runIndex: number;
  pointIndex: number;
  found: boolean;
  firstHour: number;
  lastHour: number;
  lastValueHour: number;
  firstValue: number;
  lastValue: number;
  minValue: number;
  maxValue: number;
  minHour: number;
  maxHour: number;
  firstBreak: boolean;
  hasBreak: boolean;
  prevLast: number;
}

function freshStraddle(
  coarse: ChartBucket,
  runs: readonly Run[] | null,
  points: readonly WitnessedPoint[] | null,
  counter?: BoundedWorkCounter,
): StraddleState {
  return {
    coarse,
    runs,
    points,
    hour: -Infinity,
    runIndex: runs && runs.length ? lowerBoundRunOverlap(runs, coarse.firstHour, counter) : 0,
    pointIndex: points && points.length ? lowerBoundPoint(points, coarse.firstHour, counter) : 0,
    found: false,
    firstHour: Infinity,
    lastHour: -Infinity,
    lastValueHour: -Infinity,
    firstValue: 0,
    lastValue: 0,
    minValue: Infinity,
    maxValue: -Infinity,
    minHour: coarse.firstHour,
    maxHour: coarse.firstHour,
    firstBreak: coarse.firstBreak,
    hasBreak: false,
    prevLast: -Infinity,
  };
}

function observeSpan(st: StraddleState, hour: number, from: number, to: number, val: number, breakBefore: boolean): void {
  const start = Math.max(st.coarse.firstHour, from);
  const end = Math.min(hour, to);
  if (start > end) return;
  if (!st.found) {
    st.found = true;
    st.firstHour = start;
    st.firstValue = val;
    st.firstBreak = st.coarse.firstBreak || breakBefore;
  } else if (breakBefore || start > st.prevLast + 1) {
    st.hasBreak = true;
  }
  if (end > st.lastHour || (end === st.lastHour && start >= st.lastValueHour)) {
    st.lastHour = end;
    st.lastValueHour = start;
    st.lastValue = val;
  }
  if (val < st.minValue) {
    st.minValue = val;
    st.minHour = start;
  }
  if (val > st.maxValue) {
    st.maxValue = val;
    st.maxHour = start;
  }
  st.prevLast = Math.max(st.prevLast, end);
}

export function cutStraddlingBucket(
  coarse: ChartBucket,
  hour: DisplayedHour,
  prefixRuns?: readonly Run[] | null,
  prefixPoints?: readonly WitnessedPoint[] | null,
  headPoint?: WitnessedPoint | null,
  counter?: BoundedWorkCounter,
  resume?: StraddleResume,
): StraddlingCutResult {
  if (coarse.firstHour > hour) {
    return { bucket: null, straddling: 'none' };
  }
  if (coarse.lastHour <= hour) {
    return { bucket: coarse, straddling: 'none' };
  }
  const runs = prefixRuns && prefixRuns.length ? prefixRuns : null;
  const points = prefixPoints && prefixPoints.length ? prefixPoints : null;
  const resumable = !!resume && !(runs && points);
  const held = resumable ? resume!.state : null;
  const st = held && held.coarse === coarse && held.runs === runs && held.points === points && held.hour <= hour
    ? held
    : freshStraddle(coarse, runs, points, counter);
  if (resumable) resume!.state = st;

  if (runs) {
    for (let i = st.runIndex; i < runs.length; i++) {
      const run = runs[i]!;
      if (counter) counter.visits++;
      if (run.firstHour > hour) break;
      observeSpan(st, hour, run.firstHour, run.lastHour, run.value, run.breakBefore === true);
      st.runIndex = i;
    }
  }
  if (points) {
    for (let i = st.pointIndex; i < points.length; i++) {
      const pt = points[i]!;
      if (counter) counter.visits++;
      if (pt.hour > hour) break;
      observeSpan(st, hour, pt.hour, pt.hour, pt.value, pt.breakBefore === true);
      st.pointIndex = i + 1;
    }
  }
  st.hour = hour;

  let out: StraddleState = st;
  if (headPoint && headPoint.hour >= coarse.firstHour && headPoint.hour <= hour) {
    if (counter) counter.visits++;
    out = { ...st };
    observeSpan(out, hour, headPoint.hour, headPoint.hour, headPoint.value, headPoint.breakBefore === true);
  }

  if (!out.found) {
    return { bucket: null, straddling: 'loading' };
  }

  if (counter) counter.allocations++;
  return {
    bucket: {
      firstHour: out.firstHour,
      lastHour: out.lastHour,
      lastValueHour: out.lastValueHour,
      minHour: out.minHour,
      maxHour: out.maxHour,
      firstValue: out.firstValue,
      lastValue: out.lastValue,
      minValue: out.minValue,
      maxValue: out.maxValue,
      firstBreak: out.firstBreak,
      hasBreak: out.hasBreak,
    },
    straddling: 'cut',
  };
}

export interface SlicedSpanResult<T> {
  readonly slice: BoundedSlice<T>;
  readonly validLo: number;
  readonly validHi: number;
}

export function sliceSpansByHour<T extends { readonly firstHour: number; readonly lastHour: number }, U = T>(
  sorted: readonly T[],
  hour: DisplayedHour,
  clipTail: (item: T, h: DisplayedHour) => U | undefined,
  counter?: BoundedWorkCounter,
): SlicedSpanResult<T | U> {
  if (!sorted.length) {
    return { slice: emptySlice<T | U>(), validLo: -Infinity, validHi: Infinity };
  }
  const count = upperBoundFirst(sorted, hour, counter);
  const nextItem = count < sorted.length ? sorted[count] : undefined;
  const nextStart = nextItem ? nextItem.firstHour - 1 : Infinity;

  if (count === 0) {
    return { slice: emptySlice<T | U>(), validLo: -Infinity, validHi: nextStart };
  }

  const last = sorted[count - 1]!;
  if (counter) counter.visits++;
  if (last.lastHour <= hour) {
    if (counter) counter.allocations++;
    return {
      slice: createBoundedSlice<T | U>(sorted, count),
      validLo: last.lastHour,
      validHi: nextStart,
    };
  }
  const tail = clipTail(last, hour);
  if (counter) counter.allocations++;
  return {
    slice: createBoundedSlice<T | U>(sorted, count - 1, tail),
    validLo: hour,
    validHi: hour,
  };
}

function clipSingleMissing(span: MissingSpan, hour: DisplayedHour): MissingSpan {
  return { ...span, lastHour: hour };
}

function clipSingleRun(run: Run, hour: DisplayedHour): Run {
  return { ...run, lastHour: hour };
}

const markTotals = new WeakMap<MarkCluster, Float64Array>();

function markPrefixTotals(cluster: MarkCluster, bins: readonly (readonly [number, number])[]): Float64Array {
  let totals = markTotals.get(cluster);
  if (!totals) {
    totals = new Float64Array(bins.length + 1);
    for (let i = 0; i < bins.length; i++) totals[i + 1] = totals[i]! + bins[i]![1];
    markTotals.set(cluster, totals);
  }
  return totals;
}

function clipSingleMark(cluster: MarkCluster, hour: DisplayedHour): BoundedMarkCluster | undefined {
  const sourceBins = cluster.bins;
  if (!sourceBins?.length) return undefined;
  let lo = 0;
  let hi = sourceBins.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (sourceBins[mid]![0] <= hour) lo = mid + 1;
    else hi = mid;
  }
  if (lo === 0) return undefined;
  const total = markPrefixTotals(cluster, sourceBins)[lo]!;
  return {
    firstHour: cluster.firstHour,
    lastHour: Math.min(cluster.lastHour, sourceBins[lo - 1]![0]),
    firstOrdinal: cluster.firstOrdinal,
    count: String(total),
    bins: createBoundedSlice(sourceBins, lo),
  };
}

function clipSingleHourSpan(
  span: { readonly firstHour: number; readonly lastHour: number },
  hour: DisplayedHour,
): { readonly firstHour: number; readonly lastHour: number } {
  return { firstHour: span.firstHour, lastHour: hour };
}

export function sliceMissingSpans(
  spans: readonly MissingSpan[],
  hour: DisplayedHour,
  counter?: BoundedWorkCounter,
): SlicedSpanResult<MissingSpan> {
  return sliceSpansByHour(spans, hour, clipSingleMissing, counter);
}

export function sliceMarks(
  clusters: readonly MarkCluster[],
  hour: DisplayedHour,
  counter?: BoundedWorkCounter,
): SlicedSpanResult<BoundedMarkCluster> {
  return sliceSpansByHour(clusters, hour, clipSingleMark, counter);
}

export function sliceRuns(
  runs: readonly Run[],
  hour: DisplayedHour,
  counter?: BoundedWorkCounter,
): SlicedSpanResult<Run> {
  return sliceSpansByHour(runs, hour, clipSingleRun, counter);
}

export function clipMissingSpans(spans: readonly MissingSpan[], hour: DisplayedHour): MissingSpan[] {
  return [...sliceMissingSpans(spans, hour).slice];
}

export function clipMarks(clusters: readonly MarkCluster[], hour: DisplayedHour): BoundedMarkCluster[] {
  return [...sliceMarks(clusters, hour).slice];
}

export function clipRuns(runs: readonly Run[], hour: DisplayedHour): Run[] {
  return [...sliceRuns(runs, hour).slice];
}

export interface UnrecordedLine {
  readonly summary: { readonly from: number; readonly to: number };
  readonly missing: readonly MissingSpan[];
}

export interface BoundedUnrecorded {
  readonly reason: string | null;
  readonly validInterval: readonly [lo: number, hi: number];
}

const NOTHING_UNRECORDED: BoundedUnrecorded = Object.freeze({ reason: null, validInterval: Object.freeze([-Infinity, Infinity]) as readonly [number, number] });

export function boundedUnrecorded(lines: Iterable<UnrecordedLine>, hour: number): BoundedUnrecorded {
  const h = Math.floor(hour);
  let any = false;
  let recordedFrom = Infinity;
  let lo = -Infinity;
  let hi = Infinity;
  let started = true;
  const reasons: string[] = [];
  for (const line of lines) {
    any = true;
    const { from, to } = line.summary;
    let at = from;
    let reasonAt: string | null = null;
    let spanLo = from;
    let spanHi = Infinity;
    for (const span of line.missing) {
      if (span.lastHour < at) continue;
      if (span.firstHour > at || at > to) break;
      const target = Math.min(h, to);
      if (reasonAt === null && span.firstHour <= target && target <= span.lastHour) {
        reasonAt = span.reason;
        spanLo = Math.max(from, span.firstHour);
        spanHi = span.lastHour >= to ? Infinity : span.lastHour;
      }
      at = span.lastHour + 1;
    }
    const prefixEnd = at > to ? Infinity : at - 1;
    recordedFrom = Math.min(recordedFrom, prefixEnd + 1);
    if (h < from) {
      started = false;
      hi = Math.min(hi, from - 1);
    } else if (h <= prefixEnd && reasonAt !== null) {
      reasons.push(reasonAt);
      lo = Math.max(lo, spanLo);
      hi = Math.min(hi, spanHi);
    }
  }
  if (!any) return NOTHING_UNRECORDED;
  if (h >= recordedFrom) return { reason: null, validInterval: [recordedFrom, Infinity] };
  hi = Math.min(hi, recordedFrom - 1);
  if (!started) return { reason: null, validInterval: [Math.min(lo, h), Math.max(h, hi)] };
  let best: string | null = null;
  let most = 0;
  const counts = new Map<string, number>();
  for (const reason of reasons) {
    const count = (counts.get(reason) ?? 0) + 1;
    counts.set(reason, count);
    if (count > most) {
      best = reason;
      most = count;
    }
  }
  return { reason: best, validInterval: [lo, hi] };
}

export interface BoundSeriesOptions {
  readonly prefixRuns?: readonly Run[] | null;
  readonly prefixPoints?: readonly WitnessedPoint[] | null;
  readonly headPoint?: WitnessedPoint | null;
  readonly headValue?: Value;
  readonly counter?: BoundedWorkCounter;
  readonly straddle?: StraddleResume;
}

export function boundTiledSeries(
  line: TiledLine,
  hour: DisplayedHour,
  options?: BoundSeriesOptions,
): BoundedSeries {
  const counter = options?.counter;
  const binHours = line.summary.tiles?.binHours ?? 1;
  const from = line.summary.from;
  const rawBuckets = line.summary.buckets;

  let closedBuckets = 0;
  let straddlingBucket: ChartBucket | undefined;
  let straddling: BoundedSeries['straddling'] = 'none';
  let bucketValidLo = -Infinity;
  let bucketValidHi = Infinity;

  if (rawBuckets.length > 0) {
    const binIdx = Math.floor((hour - from) / binHours);
    if (binIdx < 0) {
      closedBuckets = 0;
      bucketValidHi = from - 1;
    } else if (binIdx >= rawBuckets.length) {
      closedBuckets = rawBuckets.length;
      bucketValidLo = from + rawBuckets.length * binHours;
    } else {
      const candidate = rawBuckets[binIdx];
      if (counter) counter.visits++;
      if (!candidate) {
        closedBuckets = binIdx + 1;
        const binStart = from + binIdx * binHours;
        const binEnd = binStart + binHours - 1;
        bucketValidLo = binStart;
        bucketValidHi = binEnd;
      } else if (candidate.firstHour > hour) {
        closedBuckets = binIdx;
        bucketValidLo = from + binIdx * binHours;
        bucketValidHi = candidate.firstHour - 1;
      } else if (candidate.lastHour <= hour) {
        closedBuckets = binIdx + 1;
        const binEnd = from + (binIdx + 1) * binHours - 1;
        bucketValidLo = candidate.lastHour;
        bucketValidHi = binEnd;
      } else {
        closedBuckets = binIdx;
        const cut = cutStraddlingBucket(
          candidate,
          hour,
          options?.prefixRuns ?? line.summary.runs,
          options?.prefixPoints,
          options?.headPoint,
          counter,
          options?.straddle,
        );
        straddling = cut.straddling;
        if (cut.bucket) straddlingBucket = cut.bucket;
        bucketValidLo = hour;
        bucketValidHi = hour;
      }
    }
  }

  const bucketsSlice = createBoundedSlice(rawBuckets, closedBuckets, straddlingBucket);
  const runsResult = line.summary.runs ? sliceRuns(line.summary.runs, hour, counter) : undefined;
  const missingResult = sliceMissingSpans(line.missing, hour, counter);
  const approxResult = line.summary.approximate ? sliceSpansByHour(line.summary.approximate, hour, clipSingleHourSpan, counter) : undefined;
  const marksResult = line.marks ? sliceMarks(line.marks.clusters, hour, counter) : undefined;

  const validLo = Math.max(
    bucketValidLo,
    runsResult?.validLo ?? -Infinity,
    missingResult.validLo,
    approxResult?.validLo ?? -Infinity,
    marksResult?.validLo ?? -Infinity,
  );
  const validHi = Math.min(
    bucketValidHi,
    runsResult?.validHi ?? Infinity,
    missingResult.validHi,
    approxResult?.validHi ?? Infinity,
    marksResult?.validHi ?? Infinity,
  );

  if (counter) counter.allocations++;
  return {
    tag: line.tag,
    decimals: line.decimals,
    binHours,
    from,
    buckets: bucketsSlice,
    ...(runsResult ? { runs: runsResult.slice } : {}),
    straddling,
    ...(options?.headValue ? { headValue: options.headValue } : {}),
    missing: missingResult.slice,
    ...(approxResult ? { approximate: approxResult.slice } : {}),
    marks: marksResult ? marksResult.slice : emptySlice<BoundedMarkCluster>(),
    validInterval: [validLo, Math.max(validLo, validHi)],
  };
}

export class BoundedSeriesCursor {
  private lastLine: TiledLine | null = null;
  private lastPrefixRuns: readonly Run[] | null | undefined = undefined;
  private lastPrefixPoints: readonly WitnessedPoint[] | null | undefined = undefined;
  private readonly straddle: StraddleResume = { state: null };
  private lastHeadPoint: WitnessedPoint | null | undefined = undefined;
  private lastHeadValue: Value | undefined = undefined;
  private cached: BoundedSeries | null = null;

  advance(line: TiledLine, hour: number, options?: BoundSeriesOptions): BoundedSeries {
    const h = displayedHour(hour);
    if (
      this.cached
      && this.lastLine === line
      && this.lastPrefixRuns === options?.prefixRuns
      && this.lastPrefixPoints === options?.prefixPoints
      && this.lastHeadPoint === options?.headPoint
      && this.lastHeadValue === options?.headValue
      && h >= this.cached.validInterval[0]
      && h <= this.cached.validInterval[1]
    ) {
      return this.cached;
    }
    if (this.lastLine !== line) this.straddle.state = null;
    const next = boundTiledSeries(line, h, { ...options, straddle: this.straddle });
    this.lastLine = line;
    this.lastPrefixRuns = options?.prefixRuns;
    this.lastPrefixPoints = options?.prefixPoints;
    this.lastHeadPoint = options?.headPoint;
    this.lastHeadValue = options?.headValue;
    this.cached = next;
    return next;
  }

  reset(): void {
    this.lastLine = null;
    this.cached = null;
    this.straddle.state = null;
  }
}

function wrapBoundedRowSource<TRow extends DatedLogRow>(
  source: TableRowSource<TRow>,
  hour: DisplayedHour,
): TableRowSource<TRow> {
  if (source.at !== null && source.at > hour) {
    throw new FutureDataError(`Row source ${source.identity} at hour ${source.at} exceeds displayed hour ${hour}`);
  }
  const boundedMissing = clipMissingSpans(source.missing, hour);
  return {
    identity: source.identity,
    total: source.total,
    liveCount: source.liveCount,
    at: source.at,
    missing: boundedMissing,
    refused: source.refused,
    frozen: source.frozen && source.frozen.hour <= hour ? source.frozen : null,
    sliceAt(reqHour: number, first: number, count: number): readonly LogRowSlot<TRow>[] {
      if (reqHour > hour) {
        throw new FutureDataError(`Requested row slice at hour ${reqHour} exceeds displayed hour ${hour}`);
      }
      return source.sliceAt(reqHour, first, count);
    },
  };
}

function defaultIdentityResult(kind: EntityKind, key: string, atHour: DisplayedHour): IdentityResult {
  return {
    kind,
    key,
    atHour,
    holder: { status: 'pending' },
    name: { status: 'pending' },
    validInterval: [atHour, atHour],
    nextChangeAt: Infinity,
  };
}

const DEFAULT_ART_PENDING: ArtResult = Object.freeze({ status: 'pending' });

export interface BoundedReadViewInput {
  readonly hour: number;
  readonly validInterval?: readonly [lo: number, hi: number];
  readonly revisions?: Readonly<Record<string, number>>;
  readonly panels?: ReadonlyMap<string, unknown>;
  readonly answers?: ReadonlyMap<string, FollowedAnswer | undefined>;
  readonly live?: ReadonlyMap<string, LiveValues<number | bigint | string> | null>;
  readonly refused?: ReadonlyMap<string, string>;
  readonly series?: ReadonlyMap<string, ReadonlyMap<string, BoundedSeries>>;
  readonly hovers?: ReadonlyMap<string, { readonly hour: number; readonly data: unknown }>;
  readonly rows?: ReadonlyMap<string, TableRowSource<DatedLogRow>>;
  readonly identity?: IdentityProvider;
  readonly onIdentityDemand?: IdentityDemand;
  readonly art?: ArtProvider;
  readonly onArtDemand?: ArtDemand;
  readonly ensureNonLive?: () => {
    readonly panels: ReadonlyMap<string, unknown>;
    readonly answers: ReadonlyMap<string, FollowedAnswer | undefined>;
    readonly refused: ReadonlyMap<string, string>;
  };
  readonly resolveLive?: (name: string) => LiveValues<number | bigint | string> | null | undefined;
}

const NO_REVISIONS: Readonly<Record<string, number>> = Object.freeze({});

export function createBoundedReadView(input: BoundedReadViewInput): BoundedReadView {
  const h = displayedHour(input.hour);
  const validInterval: readonly [number, number] = input.validInterval ?? [h, h];
  const revisions = input.revisions ?? NO_REVISIONS;
  const wrappedRows = new Map<string, TableRowSource<DatedLogRow>>();
  let nonLive: {
    readonly panels: ReadonlyMap<string, unknown>;
    readonly answers: ReadonlyMap<string, FollowedAnswer | undefined>;
    readonly refused: ReadonlyMap<string, string>;
  } | null = null;

  const ensure = () => {
    if (nonLive) return nonLive;
    nonLive = input.ensureNonLive
      ? input.ensureNonLive()
      : {
          panels: input.panels ?? new Map(),
          answers: input.answers ?? new Map(),
          refused: input.refused ?? new Map(),
        };
    return nonLive;
  };

  return {
    hour: h,
    validInterval,
    revisions,
    panel<T = unknown>(name: string): T | undefined {
      const raw = ensure().panels.get(name);
      if (raw !== undefined && raw !== null && typeof raw === 'object') {
        const maybeHour = (raw as { hour?: unknown }).hour;
        if (typeof maybeHour === 'number' && Number.isFinite(maybeHour) && maybeHour > h) {
          throw new FutureDataError(`Panel "${name}" carries hour ${maybeHour} after displayed hour ${h}`);
        }
      }
      return raw as T | undefined;
    },
    answer(name: string): FollowedAnswer | undefined {
      return ensure().answers.get(name);
    },
    live(name: string): LiveValues<number | bigint | string> | null | undefined {
      return input.resolveLive ? input.resolveLive(name) : input.live?.get(name);
    },
    refused(name: string): string | undefined {
      return ensure().refused.get(name);
    },
    series(name: string): ReadonlyMap<string, BoundedSeries> | undefined {
      return input.series?.get(name);
    },
    hover<T = unknown>(name: string, atHour?: number): BoundedHover<T> | undefined {
      const requestedHour = atHour !== undefined ? displayedHour(atHour) : h;
      if (requestedHour > h) {
        throw new FutureDataError(`Hover "${name}" requested at hour ${requestedHour} after displayed hour ${h}`);
      }
      const entry = input.hovers?.get(name);
      if (!entry) return undefined;
      if (entry.hour > h) {
        throw new FutureDataError(`Hover "${name}" at hour ${entry.hour} exceeds displayed hour ${h}`);
      }
      if (atHour !== undefined && entry.hour !== requestedHour) return undefined;
      return { hour: displayedHour(entry.hour), data: entry.data as T };
    },
    rows<TRow extends DatedLogRow = DatedLogRow>(name: string): TableRowSource<TRow> | undefined {
      const raw = input.rows?.get(name);
      if (!raw) return undefined;
      let held = wrappedRows.get(name);
      if (!held) {
        held = wrapBoundedRowSource(raw, h);
        wrappedRows.set(name, held);
      }
      return held as TableRowSource<TRow>;
    },
    who(kind: EntityKind, key: string, atHour?: number): IdentityResult {
      const target = atHour !== undefined ? displayedHour(atHour) : h;
      if (target > h) {
        throw new FutureDataError(`Identity lookup who(${kind}, ${key}, ${target}) exceeds displayed hour ${h}`);
      }
      if (!input.identity) {
        input.onIdentityDemand?.(kind, key, target);
        return defaultIdentityResult(kind, key, target);
      }
      const result = input.identity.resolve(kind, key, target);
      if (
        input.onIdentityDemand
        && (
          result.holder.status === 'pending'
          || result.name.status === 'pending'
          || result.tag?.status === 'pending'
          || result.colour?.status === 'pending'
          || result.flagChoice?.status === 'pending'
        )
      ) {
        input.onIdentityDemand(kind, key, target);
      }
      return result;
    },
    art(handle: ArtHandle): ArtResult {
      if (!input.art) {
        input.onArtDemand?.(handle);
        return DEFAULT_ART_PENDING;
      }
      const result = input.art.resolve(handle);
      if (result.status === 'pending') {
        input.onArtDemand?.(handle);
      }
      return result;
    },
  };
}

export function emptyReadView(hour: number): BoundedReadView {
  return createBoundedReadView({ hour });
}
