import type { CountryIdentityIndex } from '../../dashboard/countryIdentity.ts';
import { selectedCountryKeys } from '../../dashboard/countrySelection.ts';
import type { BoundTrack, CountrySelection } from '../../dashboard/model';
import type { PlaybackController } from '../../dashboard/playback';
import { watchBoundary } from '../../dashboard/widgetTime.ts';
import { groupSelection } from '../../families/groupSelection.ts';
import type { HourRanges } from '../../families/holders.ts';
import { resolveSelectedCountryRefs } from '../../families/table/logHelpers.ts';
import type { CountryRef, EventFeedActivityReply, GroupSelection, MissingSpan } from '../../generated/dashboard';

const STRIP_WINDOW_HOURS = 60 * 24;
const DEFAULT_BIN_HOURS = 24;

export type ActivityCountryFilter =
  | { readonly kind: 'all' }
  | { readonly kind: 'group'; readonly group: GroupSelection }
  | { readonly kind: 'countries'; readonly countries: readonly CountryRef[] }
  | { readonly kind: 'empty' };

export interface StripState {
  readonly key: string;
  readonly scopeKey: string;
  readonly boundFirstHour: number;
  readonly firstHour: number;
  readonly lastHour: number;
  readonly binHours: number;
  readonly binCount: number;
  readonly beforeByCategory: Readonly<Record<string, bigint>>;
  readonly series: readonly { readonly category: string; readonly counts: readonly number[] }[];
  readonly missing: readonly MissingSpan[];
}

export interface ActivityRequestPlan {
  readonly queryKey: string;
  readonly scopeKey: string;
  readonly boundFirstHour: number;
  readonly boundLastHour: number;
  readonly binHours: number;
  readonly requestFrom: number;
  readonly incremental: boolean;
  readonly invalidatePrev: boolean;
}

export interface ActivityDisplayTarget {
  readonly queryKey: string;
  readonly scopeKey: string;
  readonly binHours: number;
  readonly bound: { readonly firstHour: number; readonly lastHour: number };
}

export interface ActivityClockTracker {
  readonly read: (axis: number) => number;
  readonly span: (axis: number) => readonly [number, number];
  readonly reset: () => void;
}

export function stripBinHours(spanHours: number): number {
  if (spanHours <= 72 * 24) return DEFAULT_BIN_HOURS;
  const days = Math.max(1, Math.ceil(spanHours / 24 / 60));
  return days * 24;
}

export function activityStripStart(
  branchFirstHour: number,
  from: number | null,
  displayedHour: number,
): number {
  if (from !== null) return Math.max(branchFirstHour, from);
  const rolling = Math.floor((displayedHour - STRIP_WINDOW_HOURS) / DEFAULT_BIN_HOURS) * DEFAULT_BIN_HOURS;
  return Math.max(branchFirstHour, rolling);
}

export function activityDisplayedHour(
  track: Pick<BoundTrack, 'axisOffset' | 'branch'> | undefined,
  pinnedHour: number | null | undefined,
  shown: number,
  frozenHour: number | null | undefined = null,
  installedHour: number | null | undefined = null,
  liveClockHour: number = shown,
): number | null {
  if (!track) return null;
  const shownTrack = Math.floor(shown + track.axisOffset);
  const liveTrack = Math.floor(liveClockHour + track.axisOffset);
  const installed = installedHour != null && Number.isFinite(installedHour) ? Math.floor(installedHour) : null;
  const follow = installed !== null && installed > shownTrack && installed <= liveTrack ? installed : shownTrack;
  const raw = pinnedHour != null
    ? Math.floor(pinnedHour)
    : frozenHour != null && Number.isFinite(frozenHour) && frozenHour <= follow
      ? Math.floor(frozenHour)
      : follow;
  if (!Number.isFinite(raw)) return null;
  if (raw < track.branch.firstHour) return null;
  return Math.min(track.branch.lastHour, raw);
}

function activityBoundarySpan(
  track: Pick<BoundTrack, 'axisOffset' | 'branch'>,
  from: number | null,
  frozenHour: number | null | undefined,
  groupGaps: HourRanges | undefined,
  axis: number,
): readonly [number, number] {
  const offset = track.axisOffset;
  const live = Math.floor(axis) + offset;
  const first = track.branch.firstHour;
  const last = track.branch.lastHour;
  if (live < first) return [-Infinity, first - offset];
  const frozen = frozenHour != null && Number.isFinite(frozenHour) ? Math.floor(frozenHour) : null;
  if (frozen !== null && live >= frozen) return [frozen - offset, Infinity];
  if (live > last) return [last + 1 - offset, Infinity];

  const stripFrom = activityStripStart(first, from, live);
  const binHours = stripBinHours(live - stripFrom);
  const binStart = Math.floor(live / binHours) * binHours;
  let low = Math.max(first, binStart);
  let high = Math.min(last + 1, binStart + binHours);
  if (from === null) {
    const dayStart = Math.floor(live / DEFAULT_BIN_HOURS) * DEFAULT_BIN_HOURS;
    low = Math.max(low, dayStart);
    high = Math.min(high, dayStart + DEFAULT_BIN_HOURS);
  } else {
    const days = binHours / DEFAULT_BIN_HOURS;
    const prevSpan = days <= 1 ? 0 : days === 2 ? 72 * 24 + 1 : (days - 1) * 60 * 24 + 1;
    const nextSpan = days <= 1 ? 72 * 24 + 1 : days * 60 * 24 + 1;
    if (prevSpan > 0) low = Math.max(low, stripFrom + prevSpan);
    high = Math.min(high, stripFrom + nextSpan);
  }
  if (frozen !== null && frozen > live) high = Math.min(high, frozen);
  for (const gap of track.branch.gaps ?? []) {
    for (const edge of [gap.firstHour, gap.lastHour + 1]) {
      if (edge <= live) low = Math.max(low, edge);
      else high = Math.min(high, edge);
    }
  }
  if (groupGaps) {
    for (const [lo, hi] of groupGaps) {
      for (const edge of [lo, hi + 1]) {
        if (edge <= live) low = Math.max(low, edge);
        else high = Math.min(high, edge);
      }
    }
  }
  return [low - offset, high - offset];
}

export function createActivityClockTracker(
  track: Pick<BoundTrack, 'axisOffset' | 'branch'> | undefined,
  from: number | null,
  frozenHour: number | null | undefined,
  groupGaps: HourRanges | undefined,
): ActivityClockTracker {
  let cached: { readonly axis: number; readonly span: readonly [number, number] } | null = null;
  const reset = () => { cached = null; };
  const read = (axis: number): number => {
    const floored = Math.floor(axis);
    if (!track || !Number.isFinite(floored)) return floored;
    if (cached && floored >= cached.span[0] && floored < cached.span[1]) return cached.axis;
    const span = activityBoundarySpan(track, from, frozenHour, groupGaps, floored);
    cached = { axis: floored, span };
    return floored;
  };
  const span = (axis: number): readonly [number, number] => {
    if (!track || !Number.isFinite(axis)) return [-Infinity, Infinity];
    const floored = Math.floor(axis);
    if (cached && floored >= cached.span[0] && floored < cached.span[1]) return cached.span;
    return activityBoundarySpan(track, from, frozenHour, groupGaps, floored);
  };
  return { read, span, reset };
}

export function subscribeActivityClock(
  clock: PlaybackController,
  tracker: ActivityClockTracker,
  active: boolean,
  changed: () => void,
): () => void {
  if (!active) return () => {};
  const initial = clock.getSnapshot();
  let lastHour = initial.hour;
  let lastSeek = initial.seekSequence;
  const stopBoundary = watchBoundary(clock, tracker.span, () => {
    const snap = clock.getSnapshot();
    lastHour = snap.hour;
    lastSeek = snap.seekSequence;
    tracker.reset();
    changed();
  });
  const stopAnchor = clock.subscribe(() => {
    const snap = clock.getSnapshot();
    if (snap.hour === lastHour && snap.seekSequence === lastSeek) return;
    lastHour = snap.hour;
    lastSeek = snap.seekSequence;
    tracker.reset();
    changed();
  });
  return () => {
    stopBoundary();
    stopAnchor();
  };
}

export function activityCountryFilter(
  selection: CountrySelection,
  track: BoundTrack | undefined,
  identities: CountryIdentityIndex | null = null,
): ActivityCountryFilter {
  if (selection.kind === 'all') return { kind: 'all' };
  const group = groupSelection(selection);
  if (group) return { kind: 'group', group };
  if (selection.kind === 'tags' || selection.kind === 'selected') {
    const keys = selectedCountryKeys(selection, track);
    const refs = resolveSelectedCountryRefs(selection, keys, identities) ?? [];
    if (!refs.length) return { kind: 'empty' };
    return { kind: 'countries', countries: refs };
  }
  return { kind: 'empty' };
}

export function activityScopeKey(input: {
  readonly sourceId: string;
  readonly revision: string;
  readonly branchId: string;
  readonly binHours: number;
  readonly categories: readonly string[];
  readonly excludedKinds?: readonly string[];
  readonly countryFilter: ActivityCountryFilter;
}): string {
  const excluded = input.excludedKinds?.length ? [...input.excludedKinds].sort() : [];
  return JSON.stringify([
    input.sourceId,
    input.revision,
    input.branchId,
    input.binHours,
    input.categories,
    excluded,
    input.countryFilter,
  ]);
}

export function canDisplayStrip(
  strip: StripState | null,
  target: ActivityDisplayTarget | null,
  failedKey: string | null = null,
): strip is StripState {
  if (!strip || strip.binCount === 0 || !target) return false;
  if (failedKey !== null && failedKey === target.queryKey) return false;
  if (strip.key === target.queryKey) return true;
  return strip.scopeKey === target.scopeKey
    && strip.binHours === target.binHours
    && target.bound.lastHour >= strip.lastHour
    && target.bound.firstHour >= strip.boundFirstHour
    && target.bound.firstHour <= strip.lastHour;
}

export function shouldFetchActivityStrip(
  retained: StripState | null,
  target: ActivityDisplayTarget | null,
): boolean {
  if (!target) return false;
  return retained?.key !== target.queryKey;
}

export function activityRequestPlan(
  prev: StripState | null,
  scopeKey: string,
  boundFirstHour: number,
  boundLastHour: number,
  binHours: number,
): ActivityRequestPlan {
  const queryKey = JSON.stringify([scopeKey, boundFirstHour, boundLastHour]);
  if (!prev) {
    return {
      queryKey,
      scopeKey,
      boundFirstHour,
      boundLastHour,
      binHours,
      requestFrom: boundFirstHour,
      incremental: false,
      invalidatePrev: false,
    };
  }
  const scopeChanged = prev.scopeKey !== scopeKey || prev.binHours !== binHours;
  const soughtBack = boundLastHour < prev.lastHour || boundFirstHour < prev.boundFirstHour;
  const disjointForward = boundFirstHour > prev.lastHour;
  if (scopeChanged || soughtBack || disjointForward) {
    return {
      queryKey,
      scopeKey,
      boundFirstHour,
      boundLastHour,
      binHours,
      requestFrom: boundFirstHour,
      incremental: false,
      invalidatePrev: true,
    };
  }
  if (prev.binCount > 0) {
    const sameStart = boundFirstHour === prev.boundFirstHour;
    const alignedSlide = boundFirstHour > prev.boundFirstHour
      && boundFirstHour % prev.binHours === 0
      && (boundFirstHour - prev.firstHour) % prev.binHours === 0;
    if (sameStart || alignedSlide) {
      const shiftBins = sameStart ? 0 : Math.round((boundFirstHour - prev.firstHour) / prev.binHours);
      const lastBinStart = prev.firstHour + (prev.binCount - 1) * prev.binHours;
      const requestFrom = Math.max(boundFirstHour, lastBinStart);
      const reusedBins = prev.binCount - 1 - shiftBins;
      if (reusedBins > 0 && requestFrom > boundFirstHour && requestFrom <= boundLastHour) {
        return {
          queryKey,
          scopeKey,
          boundFirstHour,
          boundLastHour,
          binHours,
          requestFrom,
          incremental: true,
          invalidatePrev: false,
        };
      }
    }
  }
  return {
    queryKey,
    scopeKey,
    boundFirstHour,
    boundLastHour,
    binHours,
    requestFrom: boundFirstHour,
    incremental: false,
    invalidatePrev: false,
  };
}

function parseBeforeCount(raw: string): bigint | null {
  try {
    return BigInt(raw);
  } catch {
    return null;
  }
}

function mergeMissingSpans(spans: readonly MissingSpan[]): MissingSpan[] {
  if (spans.length <= 1) return spans.map(s => ({ ...s }));
  const sorted = [...spans].sort((a, b) => a.firstHour - b.firstHour || a.lastHour - b.lastHour);
  const out: MissingSpan[] = [{ ...sorted[0]! }];
  for (let i = 1; i < sorted.length; i++) {
    const next = sorted[i]!;
    const prev = out[out.length - 1]!;
    if (prev.reason === next.reason && prev.lastHour + 1 >= next.firstHour) {
      if (next.lastHour > prev.lastHour) prev.lastHour = next.lastHour;
    } else {
      out.push({ ...next });
    }
  }
  return out;
}

export function buildFullActivityStrip(
  data: EventFeedActivityReply,
  plan: ActivityRequestPlan,
): StripState {
  const binCount = Math.max(0, Math.floor((plan.boundLastHour - data.firstHour) / data.binHours) + 1);
  const beforeByCategory: Record<string, bigint> = {};
  const series = data.lines.map(line => {
    beforeByCategory[line.category] = parseBeforeCount(line.before) ?? 0n;
    const counts = new Array<number>(binCount).fill(0);
    for (const [at, count] of line.bins) {
      if (at >= 0 && at < binCount) counts[at] = count;
    }
    return { category: line.category, counts };
  });
  return {
    key: plan.queryKey,
    scopeKey: plan.scopeKey,
    boundFirstHour: plan.boundFirstHour,
    firstHour: data.firstHour,
    lastHour: plan.boundLastHour,
    binHours: data.binHours,
    binCount,
    beforeByCategory,
    series,
    missing: data.missing,
  };
}

export function mergeActivityStrip(
  prev: StripState | null,
  data: EventFeedActivityReply,
  plan: ActivityRequestPlan,
): StripState | null {
  if (!plan.incremental) return buildFullActivityStrip(data, plan);
  if (
    !prev
    || prev.scopeKey !== plan.scopeKey
    || data.binHours !== prev.binHours
    || data.firstHour < prev.firstHour
    || (data.firstHour - prev.firstHour) % prev.binHours !== 0
  ) {
    return null;
  }
  const sameStart = plan.boundFirstHour === prev.boundFirstHour;
  const shiftBins = sameStart ? 0 : Math.round((plan.boundFirstHour - prev.firstHour) / prev.binHours);
  if (shiftBins < 0 || (!sameStart && (plan.boundFirstHour - prev.firstHour) % prev.binHours !== 0)) {
    return null;
  }
  const newFirstHour = prev.firstHour + shiftBins * prev.binHours;
  const splitBin = Math.round((data.firstHour - newFirstHour) / prev.binHours);
  const prevSplitBin = shiftBins + splitBin;
  if (splitBin <= 0 || prevSplitBin > prev.binCount) {
    return null;
  }
  const binCount = Math.max(0, Math.floor((plan.boundLastHour - newFirstHour) / prev.binHours) + 1);
  const prevByCategory = new Map(prev.series.map(s => [s.category, s.counts]));
  if (data.lines.length !== prev.series.length) {
    return null;
  }
  const nextBeforeByCategory: Record<string, bigint> = {};
  const series: { readonly category: string; readonly counts: readonly number[] }[] = [];
  for (const line of data.lines) {
    const prevCounts = prevByCategory.get(line.category);
    const prevBefore = prev.beforeByCategory[line.category];
    const replyBefore = parseBeforeCount(line.before);
    if (!prevCounts || prevBefore === undefined || replyBefore === null) {
      return null;
    }
    let droppedSum = 0n;
    for (let i = 0; i < shiftBins && i < prevCounts.length; i++) {
      droppedSum += BigInt(prevCounts[i]!);
    }
    let keptPrefixSum = 0n;
    for (let i = shiftBins; i < prevSplitBin && i < prevCounts.length; i++) {
      keptPrefixSum += BigInt(prevCounts[i]!);
    }
    if (prevBefore + droppedSum + keptPrefixSum !== replyBefore) {
      return null;
    }
    nextBeforeByCategory[line.category] = prevBefore + droppedSum;
    const counts = new Array<number>(binCount).fill(0);
    const copyLimit = Math.min(splitBin, binCount);
    for (let i = 0; i < copyLimit; i++) {
      counts[i] = prevCounts[shiftBins + i] ?? 0;
    }
    for (const [at, count] of line.bins) {
      const target = splitBin + at;
      if (target >= 0 && target < binCount) counts[target] = count;
    }
    series.push({ category: line.category, counts });
  }
  const prefixMissing: MissingSpan[] = [];
  for (const span of prev.missing) {
    if (span.firstHour >= plan.requestFrom) continue;
    const first = Math.max(span.firstHour, plan.boundFirstHour);
    const last = Math.min(span.lastHour, plan.requestFrom - 1);
    if (first <= last) prefixMissing.push({ firstHour: first, lastHour: last, reason: span.reason });
  }
  const missing = mergeMissingSpans([...prefixMissing, ...data.missing]);
  return {
    key: plan.queryKey,
    scopeKey: plan.scopeKey,
    boundFirstHour: plan.boundFirstHour,
    firstHour: newFirstHour,
    lastHour: plan.boundLastHour,
    binHours: prev.binHours,
    binCount,
    beforeByCategory: nextBeforeByCategory,
    series,
    missing,
  };
}

export function stripBinCoverage(
  firstHour: number,
  boundFirstHour: number,
  lastHour: number,
  binHours: number,
  binCount: number,
  missing: readonly MissingSpan[],
  extraUnobserved?: HourRanges,
): { readonly gapBins: Set<number>; readonly partBins: Set<number> } {
  const gapBins = new Set<number>();
  const partBins = new Set<number>();
  if (binCount <= 0 || binHours <= 0 || boundFirstHour > lastHour) {
    return { gapBins, partBins };
  }
  const clipped: [number, number][] = [];
  const push = (lo: number, hi: number) => {
    const start = Math.max(lo, boundFirstHour);
    const end = Math.min(hi, lastHour);
    if (start <= end) clipped.push([start, end]);
  };
  for (const span of missing) push(span.firstHour, span.lastHour);
  if (extraUnobserved) {
    for (const [lo, hi] of extraUnobserved) push(lo, hi);
  }
  if (!clipped.length) return { gapBins, partBins };
  clipped.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  const unread: [number, number][] = [clipped[0]!];
  for (let i = 1; i < clipped.length; i++) {
    const span = clipped[i]!;
    const held = unread[unread.length - 1]!;
    if (span[0] <= held[1] + 1) held[1] = Math.max(held[1], span[1]);
    else unread.push(span);
  }
  const missingHours = new Array<number>(binCount).fill(0);
  const binStart = (idx: number) => Math.max(firstHour + idx * binHours, boundFirstHour);
  const binEnd = (idx: number) => Math.min(firstHour + (idx + 1) * binHours - 1, lastHour);
  for (const [lo, hi] of unread) {
    const firstIdx = Math.max(0, Math.min(binCount - 1, Math.floor((lo - firstHour) / binHours)));
    const lastIdx = Math.max(firstIdx, Math.min(binCount - 1, Math.floor((hi - firstHour) / binHours)));
    for (let idx = firstIdx; idx <= lastIdx; idx++) {
      missingHours[idx]! += Math.max(0, Math.min(hi, binEnd(idx)) - Math.max(lo, binStart(idx)) + 1);
    }
  }
  for (let idx = 0; idx < binCount; idx++) {
    if (missingHours[idx]! <= 0) continue;
    if (missingHours[idx]! >= binEnd(idx) - binStart(idx) + 1) gapBins.add(idx);
    else partBins.add(idx);
  }
  return { gapBins, partBins };
}
