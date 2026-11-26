import type { Span } from '../../charts/series';
import { runAt } from '../../charts/series.ts';
import { pausedChartCutoff } from '../../dashboard/regionTime.ts';
import type { FuelAwardPart, FuelReadAt } from '../../generated/dashboard';

export { pausedChartCutoff };

export interface FuelBasis { branchId: string; tag: string; hour: number; country: number; countryInstance: number }

export function readBasis(branchId: string, tag: string, read: Pick<FuelReadAt, 'readHour' | 'country' | 'countryInstance'> | null | undefined): FuelBasis | null {
  return read && read.readHour !== null && read.countryInstance !== null
    ? { branchId, tag, hour: read.readHour, country: read.country, countryInstance: read.countryInstance } : null;
}

export interface FuelRecipientSelection { campaign: string; branch: string; consumer: number; basis: FuelBasis; key: string; name: string | null }

export function selectFuelRecipient(campaign: string, basis: FuelBasis, part: FuelAwardPart): FuelRecipientSelection {
  return { campaign, branch: basis.branchId, consumer: part.kind, basis, key: part.key, name: part.recordedName };
}

export function fuelRecipientFor<T extends { campaign: string; branch: string; basis: FuelBasis }>(selection: T | null | undefined, campaign: string, branch: string,
  tag: string, aliases?: ReadonlyMap<string, readonly Span[]>): T | null {
  return selection?.campaign === campaign && selection.branch === branch && selection.basis.branchId === branch
    && (selection.basis.tag === tag && !aliases || !!runAt(aliases?.get(selection.basis.tag) ?? [], selection.basis.hour) && !!aliases?.get(tag)?.length) ? selection : null;
}

export function detailWindows(extent: readonly [number, number], visible: readonly [number, number], hour: number | null, detailHours: number): number[] {
  const from = Math.max(extent[0], visible[0]), to = Math.min(extent[1], visible[1]);
  const wanted = new Set<number>();
  for (let window = Math.floor(from / detailHours); window <= Math.floor(to / detailHours); window++) wanted.add(window);
  if (hour !== null && hour >= extent[0] && hour <= extent[1]) wanted.add(Math.floor(hour / detailHours));
  return [...wanted].sort((a, b) => a - b);
}

export function joinedRuns<T extends { firstHour: number; lastHour: number }>(windows: readonly (readonly T[])[]): T[] {
  const runs = new Map<number, T>();
  for (const list of windows) for (const run of list) {
    const previous = runs.get(run.firstHour);
    if (!previous || run.lastHour > previous.lastHour) runs.set(run.firstHour, run);
  }
  return [...runs.values()].sort((a, b) => a.firstHour - b.firstHour);
}

export function intersectSpans(
  a: readonly Span[] | null | undefined,
  b: readonly Span[] | null | undefined,
): readonly Span[] | null {
  if (!a && !b) return null;
  if (!a) return b ?? null;
  if (!b) return a;
  const out: Span[] = [];
  for (const sa of a) {
    for (const sb of b) {
      const firstHour = Math.max(sa.firstHour, sb.firstHour);
      const lastHour = Math.min(sa.lastHour, sb.lastHour);
      if (firstHour <= lastHour) out.push({ firstHour, lastHour });
    }
  }
  out.sort((x, y) => x.firstHour - y.firstHour || x.lastHour - y.lastHour);
  const merged: Span[] = [];
  for (const span of out) {
    const prev = merged.at(-1);
    if (prev && span.firstHour <= prev.lastHour + 1) {
      merged[merged.length - 1] = { firstHour: prev.firstHour, lastHour: Math.max(prev.lastHour, span.lastHour) };
    } else {
      merged.push(span);
    }
  }
  return merged;
}

export function memberSpansForAliases(
  aliases: ReadonlyMap<string, readonly Span[]> | null | undefined,
  membership: ReadonlyMap<string, readonly Span[]> | null | undefined,
  fallbackTag: string | null,
): readonly Span[] | null {
  if (!membership) return null;
  if (!aliases) return fallbackTag ? membership.get(fallbackTag) ?? [] : [];
  const pieces: Span[] = [];
  for (const [aliasTag, aliasSpans] of aliases) {
    const memberSpans = membership.get(aliasTag) ?? [];
    const clipped = intersectSpans(aliasSpans, memberSpans);
    if (clipped) pieces.push(...clipped);
  }
  pieces.sort((x, y) => x.firstHour - y.firstHour || x.lastHour - y.lastHour);
  const merged: Span[] = [];
  for (const span of pieces) {
    const prev = merged.at(-1);
    if (prev && span.firstHour <= prev.lastHour + 1) {
      merged[merged.length - 1] = { firstHour: prev.firstHour, lastHour: Math.max(prev.lastHour, span.lastHour) };
    } else {
      merged.push(span);
    }
  }
  return merged;
}

export function followedHistoryAnchor(
  extent: readonly [number, number],
  liveHour: number,
  detailHours: number,
): number {
  const step = Math.max(24, Math.floor(detailHours / 4));
  const quantized = Math.floor(liveHour / step) * step + step - 1;
  return Math.max(extent[0], Math.min(extent[1], quantized));
}
