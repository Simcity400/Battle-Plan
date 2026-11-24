import { colour, nudgeApart, readableColour } from '../../charts/palette.ts';
import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import { rowName, type FollowedAnswer } from '../../dashboard/follow/message.ts';
import { rowAmount } from '../../dashboard/follow/painters.ts';
import { flagArtHandle } from '../art/provider.ts';
import { failedArt, missingArt, PENDING_ART } from '../art/types.ts';
import { FutureDataError, type BoundedReadView, type BoundedSeries, type FamilyPlanContext, type SourceEntry } from '../contract.ts';
import { holderTagAt, type HolderRun, type HourRanges } from '../holders.ts';
import { parseColor, toHex } from '../../shell/appearance/color.ts';
import type { TooltipListItem } from '../host/tooltips.ts';
import type { ChartArtSlot, ChartEndLabel, ChartGap, ChartMarker, ChartMissingSpan, ChartStackBand, ChartStackModel } from './model.ts';

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
    hover = readView.hover<readonly ExactState[] | { readonly states?: readonly ExactState[]; readonly decimals?: number }>(name);
  } catch (error) {
    if (error instanceof FutureDataError) return null;
    throw error;
  }
  if (!hover || !hover.data) return null;
  const data = hover.data;
  const list: readonly ExactState[] | null = Array.isArray(data)
    ? data
    : 'states' in data && Array.isArray(data.states)
      ? data.states
      : null;
  if (!list) return null;
  const usedDecimals = !Array.isArray(data) && 'decimals' in data && typeof data.decimals === 'number' ? data.decimals : decimals;
  const divisor = 10 ** usedDecimals;
  return list.map(state => {
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
  recordedOrdinal: (tag: string) => number = tag => orderedTags.indexOf(tag),
): ReadonlyMap<string, string> {
  const map = new Map<string, string>();
  if (orderedTags.length === 1 && singleColor) {
    map.set(orderedTags[0]!, singleColor);
    return map;
  }
  const top = orderedTags.slice(0, 8);
  const bg = parseColor(surface) ?? { r: 17, g: 17, b: 17 };
  const rawRgbs = top.map(tag => {
    const identityTag = holderTagAt(holders?.[tag], hour) ?? tag;
    const who = readView.who('country', identityTag);
    const hex = who.colour?.status === 'ready'
      ? readableColour(who.colour.value, surface)
      : colour(surface, recordedOrdinal(tag));
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

export function memberAt(
  membership: Readonly<Record<string, HourRanges>> | null | undefined,
  tag: string,
  hour: number,
): boolean {
  if (!membership) return true;
  const ranges = membership[tag];
  if (!ranges) return false;
  for (const [first, last] of ranges) {
    if (hour >= first && hour <= last) return true;
  }
  return false;
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

export interface CardRow {
  readonly label: string;
  readonly value?: string;
  readonly tone?: TooltipListItem['tone'];
  readonly strong?: boolean;
  readonly swatch?: string;
  readonly divider?: boolean;
}

export function cardItems(rows: readonly CardRow[], heading: boolean): TooltipListItem[] {
  return rows.map((row, index) => ({
    label: row.label,
    ...(row.value !== undefined ? { value: row.value } : {}),
    ...(row.tone ? { tone: row.tone } : {}),
    ...(row.strong ? { strong: true } : {}),
    ...(row.swatch ? { swatch: row.swatch } : {}),
    ...(row.divider ? { divider: true } : {}),
    ...(heading && index > 0 && !row.strong ? { indent: true } : {}),
  }));
}

interface StatesReply {
  readonly states?: readonly { readonly country: string | number; readonly run: { readonly valueRaw: string } | null; readonly missing: string | null }[];
}

export function seriesHeadAt(
  readView: BoundedReadView,
  name: string,
  tag: string,
  row: number,
  defaultDecimals: number,
  scale = 1,
): number | null {
  const bounded = readView.series(name)?.get(tag) ?? null;
  const answer = readView.answer(name);
  if (answer || bounded) return seriesHeadValue(bounded, answer, row, defaultDecimals, scale);
  const reply = readView.panel<StatesReply>(name);
  const state = reply?.states?.find(item => countryKey(item.country) === tag);
  if (!state?.run) return null;
  const value = Number(state.run.valueRaw) / 10 ** defaultDecimals;
  return Number.isFinite(value) ? value * scale : null;
}

export function rankEntry(
  ctx: Pick<FamilyPlanContext, 'track' | 'branch' | 'following'>,
  family: Readonly<Record<string, unknown>>,
  tags: readonly string[],
  name = 'rank',
): SourceEntry | null {
  if (tags.length < 2) return null;
  const coverage = { mode: 'population', form: 'top', limit: tags.length } as const;
  const base = { branchId: ctx.branch.id, family, top: tags.length };
  const common = { name, source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset };
  if (ctx.following) {
    return {
      ...common,
      read: {
        shape: 'point',
        channel: 'followed',
        coverage,
        product: 'series-state',
        followQuery: { product: 'series-state', params: { ...base, reads: tags.map(tag => ({ country: countryFromKey(tag) })) } },
        params: null,
      },
    };
  }
  return {
    ...common,
    read: {
      shape: 'point',
      channel: 'panel',
      coverage,
      product: 'series-state',
      exactProduct: 'series-state',
      followQuery: null,
      params: base,
      buildExactParams: (params, hour) => ({ ...params, reads: tags.map(tag => ({ country: countryFromKey(tag), hour })) }),
    },
  };
}

export function rankedOrder(readView: BoundedReadView, tags: readonly string[], name = 'rank'): readonly string[] {
  const ranked: string[] = [];
  const answer = readView.answer(name);
  if (answer && answer.kind !== 'notRecorded') {
    for (let row = 0; row < answer.rows; row++) {
      const key = rowName(answer, row);
      if (key) ranked.push(key);
    }
  } else {
    for (const state of readView.panel<StatesReply>(name)?.states ?? []) ranked.push(countryKey(state.country));
  }
  if (ranked.length === 0) return tags;
  const selected = new Set(tags);
  const seen = new Set<string>();
  const order: string[] = [];
  for (const key of ranked) {
    if (selected.has(key) && !seen.has(key)) {
      seen.add(key);
      order.push(key);
    }
  }
  for (const tag of tags) if (!seen.has(tag)) order.push(tag);
  return order;
}

export interface BoundedStackBandInput {
  readonly id: string;
  readonly key: string;
  readonly name: string;
  readonly color: string;
  readonly opacity?: number;
  readonly bounded?: BoundedSeries | null;
  readonly parts?: readonly BoundedSeries[];
  readonly valueScale?: number;
  readonly headValue?: number | null;
  readonly endLabel?: ChartEndLabel | null;
  readonly flag?: ChartArtSlot;
}

export interface BoundedStackOptions {
  readonly missingMode?: 'all-required' | 'zero-absent';
}

function inMissing(series: BoundedSeries, hour: number): boolean {
  for (const span of series.missing) {
    if (hour >= span.firstHour && hour <= span.lastHour) return true;
  }
  return false;
}

function sampleOneBounded(series: BoundedSeries, hour: number, scale: number): { readonly value: number | null; readonly missing: boolean } {
  if (inMissing(series, hour)) return { value: null, missing: true };
  if (series.runs && series.runs.length > 0) {
    let lo = 0;
    let hi = series.runs.length - 1;
    while (lo <= hi) {
      const mid = (lo + hi) >>> 1;
      const run = series.runs.at(mid)!;
      if (hour < run.firstHour) hi = mid - 1;
      else if (hour > run.lastHour) lo = mid + 1;
      else return { value: Number.isFinite(run.value) ? run.value * scale : null, missing: false };
    }
    return { value: null, missing: false };
  }
  const idx = Math.floor((hour - series.from) / series.binHours);
  if (idx < 0 || idx >= series.buckets.length) return { value: null, missing: false };
  const b = series.buckets.at(idx);
  if (!b || b.hasBreak || hour < b.firstHour || hour > b.lastHour) return { value: null, missing: false };
  const raw = hour === b.lastHour ? b.lastValue : b.firstValue;
  return { value: Number.isFinite(raw) ? raw * scale : null, missing: false };
}

function collectSeriesHours(series: BoundedSeries, until: number, target: Set<number>): void {
  if (series.runs && series.runs.length > 0) {
    let prevLast = -Infinity;
    for (const run of series.runs) {
      if (run.firstHour > until) break;
      if (prevLast !== -Infinity && (run.breakBefore || run.firstHour > prevLast + 1)) {
        const gapHour = Math.min(until, prevLast + 1);
        if (gapHour < run.firstHour) target.add(gapHour);
      }
      target.add(run.firstHour);
      const end = Math.min(until, run.lastHour);
      if (end > run.firstHour) target.add(end);
      prevLast = end;
    }
  } else {
    let prevLast = -Infinity;
    for (const b of series.buckets) {
      if (!b || b.firstHour > until) continue;
      if (prevLast !== -Infinity && (b.firstBreak || b.firstHour > prevLast + Math.max(1, series.binHours))) {
        const gapHour = Math.min(until, prevLast + 1);
        if (gapHour < b.firstHour) target.add(gapHour);
      }
      if (b.hasBreak) {
        target.add(b.firstHour);
      } else {
        target.add(b.firstHour);
        const end = Math.min(until, b.lastHour);
        if (end > b.firstHour) target.add(end);
      }
      prevLast = Math.min(until, b.lastHour);
    }
  }
  for (const span of series.missing) {
    if (span.firstHour <= until) target.add(span.firstHour);
  }
}

export function buildBoundedStack(
  inputs: readonly BoundedStackBandInput[],
  hour: number,
  options?: BoundedStackOptions,
): ChartStackModel {
  const mode = options?.missingMode ?? 'all-required';
  const hourSet = new Set<number>();
  const sources = inputs.map(input => {
    const list: BoundedSeries[] = [];
    if (input.parts && input.parts.length > 0) list.push(...input.parts);
    else if (input.bounded) list.push(input.bounded);
    for (const s of list) collectSeriesHours(s, hour, hourSet);
    return list;
  });
  if (inputs.some(input => input.headValue !== undefined && input.headValue !== null) || hourSet.size > 0) {
    hourSet.add(hour);
  }
  const hours = [...hourSet].filter(h => h <= hour).sort((a, b) => a - b);
  const valuesByBand: (number | null)[][] = inputs.map(() => new Array<number | null>(hours.length).fill(null));

  for (let i = 0; i < hours.length; i++) {
    const h = hours[i]!;
    const row = new Array<number | null>(inputs.length).fill(null);
    let anyMissing = false;
    let anyPresent = false;
    for (let b = 0; b < inputs.length; b++) {
      const input = inputs[b]!;
      if (h === hour && input.headValue !== undefined) {
        if (input.headValue !== null && Number.isFinite(input.headValue)) {
          row[b] = input.headValue;
          anyPresent = true;
        } else {
          anyMissing = true;
        }
        continue;
      }
      const scale = input.valueScale ?? 1;
      const list = sources[b]!;
      let picked: number | null = null;
      let missing = false;
      for (const s of list) {
        const res = sampleOneBounded(s, h, scale);
        if (res.value !== null) {
          picked = res.value;
          break;
        }
        if (res.missing) missing = true;
      }
      if (picked !== null) {
        row[b] = picked;
        anyPresent = true;
      } else if (missing) {
        anyMissing = true;
      }
    }
    if (!anyPresent || anyMissing) continue;
    if (mode === 'all-required') {
      if (row.some(v => v === null)) continue;
      for (let b = 0; b < inputs.length; b++) valuesByBand[b]![i] = row[b]!;
    } else {
      for (let b = 0; b < inputs.length; b++) valuesByBand[b]![i] = row[b] ?? 0;
    }
  }

  const bands: ChartStackBand[] = inputs.map((input, idx) => ({
    id: input.id,
    key: input.key,
    name: input.name,
    color: input.color,
    ...(input.opacity !== undefined ? { opacity: input.opacity } : {}),
    values: valuesByBand[idx]!,
    ...(input.endLabel !== undefined ? { endLabel: input.endLabel } : {}),
    ...(input.flag ? { flag: input.flag } : {}),
  }));
  return { hours, bands };
}
