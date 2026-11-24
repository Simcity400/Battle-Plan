import { colour, nudgeApart, readableColour } from '../../charts/palette.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import { rowAmount } from '../../dashboard/follow/painters.ts';
import { flagArtHandle } from '../art/provider.ts';
import { failedArt, missingArt, PENDING_ART } from '../art/types.ts';
import { FutureDataError, type BoundedReadView, type BoundedSeries } from '../contract.ts';
import { holderTagAt, type HolderRun } from '../holders.ts';
import { parseColor, toHex } from '../../shell/appearance/color.ts';
import type { ChartArtSlot, ChartGap, ChartMarker, ChartMissingSpan } from './model.ts';

export function seriesHeadValue(
  bounded: BoundedSeries | null | undefined,
  answer: FollowedAnswer | undefined,
  rowIdx: number,
  defaultDecimals: number,
  scale = 1,
): number | null {
  const decimals = bounded?.decimals ?? defaultDecimals;
  if (answer) {
    const live = rowAmount(answer, rowIdx, decimals);
    if (live === null) return null;
    if (live !== undefined && Number.isFinite(live)) return live * scale;
  }
  const head = bounded?.headValue;
  if (head) {
    if (head.kind === 'notRecorded' || head.kind === 'notApplicable') return null;
    const num = Number(head.value);
    return Number.isFinite(num) ? num * scale : null;
  }
  if (!bounded) return null;
  let lastHour = -Infinity;
  let lastValue: number | null = null;
  if (bounded.runs && bounded.runs.length > 0) {
    const last = bounded.runs.at(bounded.runs.length - 1);
    if (last && Number.isFinite(last.value)) {
      lastHour = last.lastHour;
      lastValue = last.value;
    }
  } else {
    for (let b = bounded.buckets.length - 1; b >= 0; b--) {
      const bucket = bounded.buckets.at(b);
      if (bucket && Number.isFinite(bucket.lastValue)) {
        lastHour = bucket.lastHour;
        lastValue = bucket.lastValue;
        break;
      }
    }
  }
  if (lastValue === null) return null;
  const lastMissing = bounded.missing.length > 0 ? bounded.missing.at(bounded.missing.length - 1) : undefined;
  if (lastMissing && lastMissing.lastHour > lastHour) return null;
  return lastValue * scale;
}

export interface ExactReading {
  readonly hour: number;
  readonly value: number | null;
  readonly reason?: string;
}

interface ExactState {
  readonly hour: number;
  readonly run: { readonly valueRaw: string } | null;
  readonly missing: string | null;
}

export function exactHoverReadings(
  readView: BoundedReadView,
  name: string,
  decimals: number,
  scale = 1,
): readonly (ExactReading | undefined)[] | null {
  let hover;
  try {
    hover = readView.hover<readonly ExactState[]>(name);
  } catch (error) {
    if (error instanceof FutureDataError) return null;
    throw error;
  }
  if (!hover || !Array.isArray(hover.data)) return null;
  const divisor = 10 ** decimals;
  return hover.data.map(state => {
    if (!state) return undefined;
    if (!state.run) return { hour: hover.hour, value: null, reason: state.missing ?? 'Not recorded' };
    return { hour: hover.hour, value: (Number(state.run.valueRaw) / divisor) * scale };
  });
}

export function assignCountrySeriesColours(
  orderedTags: readonly string[],
  holders: Readonly<Record<string, readonly HolderRun[]>> | null | undefined,
  readView: BoundedReadView,
  hour: number,
  surface: string,
  singleColor?: string,
): ReadonlyMap<string, string> {
  const map = new Map<string, string>();
  if (orderedTags.length === 1 && singleColor) {
    map.set(orderedTags[0]!, singleColor);
    return map;
  }
  const top = orderedTags.slice(0, 8);
  const bg = parseColor(surface) ?? { r: 17, g: 17, b: 17 };
  const rawRgbs = top.map((tag, ordinal) => {
    const identityTag = holderTagAt(holders?.[tag], hour) ?? tag;
    const who = readView.who('country', identityTag);
    const hex = who.colour?.status === 'ready'
      ? readableColour(who.colour.value, surface)
      : colour(surface, ordinal);
    return parseColor(hex) ?? { r: 120, g: 160, b: 220 };
  });
  const distinct = nudgeApart(rawRgbs, bg);
  top.forEach((tag, idx) => map.set(tag, toHex(distinct[idx]!)));
  return map;
}

export function countrySeriesIdentity(
  tag: string,
  holders: Readonly<Record<string, readonly HolderRun[]>> | null | undefined,
  readView: BoundedReadView,
  hour: number,
  withFlag: boolean,
): { readonly identityTag: string; readonly name: string; readonly flag?: ChartArtSlot } {
  const identityTag = holderTagAt(holders?.[tag], hour) ?? tag;
  const who = readView.who('country', identityTag);
  const name = who.name.status === 'ready' ? who.name.value : identityTag;
  const flag: ChartArtSlot | undefined = !withFlag
    ? undefined
    : who.flagChoice?.status === 'ready'
      ? { art: readView.art(flagArtHandle(who.flagChoice.value, 'panel')), tag: identityTag }
      : who.flagChoice?.status === 'absent'
        ? { art: missingArt(who.flagChoice.reason), tag: identityTag }
        : who.flagChoice?.status === 'failed'
          ? { art: failedArt(who.flagChoice.reason, who.flagChoice.retryable), tag: identityTag }
          : { art: PENDING_ART, tag: identityTag };
  return { identityTag, name, ...(flag ? { flag } : {}) };
}

export function branchGapsOf(branch: unknown): ChartGap[] {
  const raw = (branch as { gaps?: readonly { firstHour: number; lastHour: number; reason?: string }[] } | null | undefined)?.gaps;
  return raw?.length
    ? raw.map(g => ({ firstHour: g.firstHour, lastHour: g.lastHour, reason: g.reason ?? 'Recording gap' }))
    : [];
}

export function withinMapOf(
  membership: Readonly<Record<string, readonly (readonly [number, number])[]>> | null | undefined,
): ReadonlyMap<string, readonly { readonly firstHour: number; readonly lastHour: number }[]> | undefined {
  if (!membership) return undefined;
  const entries = Object.entries(membership)
    .filter(([t]) => t.length > 0)
    .map(([t, ranges]) => [t, ranges.map(([firstHour, lastHour]) => ({ firstHour, lastHour }))] as const);
  return entries.length > 0 ? new Map(entries) : undefined;
}

export function spendMarkersOf(bounded: BoundedSeries | null | undefined, title: string): ChartMarker[] {
  if (!bounded) return [];
  const hours = new Set<number>();
  for (const cluster of bounded.marks) {
    if (cluster.bins && cluster.bins.length > 0) {
      for (const [binHour] of cluster.bins) hours.add(binHour);
    } else {
      hours.add(cluster.firstHour);
    }
  }
  return [...hours]
    .sort((a, b) => a - b)
    .map((h, idx) => ({ id: `spend:${idx}:${h}`, hour: h, title }));
}

export function missingSpansOf(bounded: BoundedSeries | null | undefined): readonly ChartMissingSpan[] | undefined {
  if (!bounded || bounded.missing.length === 0) return undefined;
  return [...bounded.missing].map(sp => ({
    firstHour: sp.firstHour,
    lastHour: sp.lastHour,
    reason: sp.reason ?? 'Not recorded',
  }));
}
