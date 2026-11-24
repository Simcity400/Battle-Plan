import type { LegendStripEntry } from '../../draw/kit.ts';
import type { HourSpan } from '../../generated/dashboard';
import { flagArtHandle } from '../art/provider.ts';
import type { ArtHandle, BoundedReadView, DisplayedHour } from '../contract.ts';
import type { TimelineLane, TimelineModel } from './model.ts';

export interface CountryLook {
  readonly tag: string;
  readonly name: string;
  readonly flag: ArtHandle | null;
  readonly colour: string | null;
}

export function countryLook(readView: BoundedReadView, tag: string, atHour: number): CountryLook {
  const who = readView.who('country', tag, Math.min(atHour, readView.hour));
  return {
    tag,
    name: who.name.status === 'ready' ? who.name.value : tag,
    flag: who.flagChoice?.status === 'ready' ? flagArtHandle(who.flagChoice.value, 'panel') : null,
    colour: who.colour?.status === 'ready' ? who.colour.value : null,
  };
}

export function clipSpansToHour(spans: readonly HourSpan[] | undefined, hour: number): HourSpan[] {
  const out: HourSpan[] = [];
  for (const span of spans ?? []) {
    if (span.firstHour > hour) continue;
    out.push({ firstHour: span.firstHour, lastHour: Math.min(span.lastHour, hour) });
  }
  return out;
}

export function inHourSpans(spans: readonly HourSpan[] | undefined, hour: number): boolean {
  for (const span of spans ?? []) {
    if (span.firstHour <= hour && hour <= span.lastHour) return true;
  }
  return false;
}

export function spanStart(firstHour: number, branchFirst: number, unobserved: readonly HourSpan[] | undefined): { readonly heldBefore?: true; readonly startBy?: true } {
  if (firstHour <= branchFirst) return { heldBefore: true };
  return inHourSpans(unobserved, firstHour - 1) ? { startBy: true } : {};
}

export function nextBoundaryAfter(hours: readonly number[], current: number): number {
  let lo = 0;
  let hi = hours.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (hours[mid]! <= current) lo = mid + 1;
    else hi = mid;
  }
  return lo < hours.length ? hours[lo]! : Infinity;
}

export interface TimelineModelParts {
  readonly hour: DisplayedHour;
  readonly firstHour: number;
  readonly lanes?: readonly TimelineLane[];
  readonly unobserved?: readonly HourSpan[];
  readonly legend?: readonly LegendStripEntry[];
  readonly iconsOnBars?: boolean;
  readonly labelWidth: number;
  readonly summaryText?: string | null;
  readonly empty?: string | null;
  readonly emptyKind?: 'empty' | 'unavailable';
  readonly warnings?: readonly string[];
  readonly notes?: readonly string[];
  readonly nextChangeAt?: number;
}

export function timelineModel(parts: TimelineModelParts): TimelineModel {
  return {
    hour: parts.hour,
    firstHour: parts.firstHour,
    lanes: parts.lanes ?? [],
    unobserved: clipSpansToHour(parts.unobserved, parts.hour),
    ...(parts.legend?.length ? { legend: parts.legend } : {}),
    iconsOnBars: parts.iconsOnBars ?? false,
    labelWidth: parts.labelWidth,
    ...(parts.summaryText ? { summaryText: parts.summaryText } : {}),
    ...(parts.empty ? { empty: parts.empty, emptyKind: parts.emptyKind ?? 'empty' } : {}),
    warnings: parts.warnings ?? [],
    ...(parts.notes?.length ? { notes: parts.notes } : {}),
    nextChangeAt: parts.nextChangeAt ?? Infinity,
  };
}

export function boundedRuns<T extends HourSpan>(runs: readonly T[], hour: number): T[] {
  const out: T[] = [];
  for (const run of runs) {
    if (run.firstHour > hour) continue;
    out.push(run.lastHour > hour ? { ...run, lastHour: hour } : run);
  }
  return out;
}

export function wholeBranchProof(_data: unknown, requested: readonly [number, number]): readonly [number, number] {
  return [requested[0], Number.MAX_SAFE_INTEGER];
}
