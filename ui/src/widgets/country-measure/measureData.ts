import type { SeriesDrop } from '../../generated/dashboard';
import { formatAmount } from '../../charts/valueAxis.ts';
import { shownClusters } from '../../charts/markSummary.ts';
import type { MarkCluster, MarkSummary } from '../../charts/markSummary';
import type { ChartSummary } from '../../charts/chartSummary';

export const formatBalance = (raw: string, decimals: number, digits = 0) => formatAmount(raw, decimals, digits);

export function changesRange(marks: MarkSummary | null | undefined, hovered: MarkCluster | null, until: number | null): { first: string; end?: string } {
  const first = hovered?.firstOrdinal ?? marks?.clusters[0]?.firstOrdinal ?? '0';
  const last = hovered ?? (marks ? shownClusters(marks, until).at(-1) : undefined);
  return { first, end: last ? (BigInt(last.firstOrdinal) + BigInt(last.count)).toString() : undefined };
}

export function netSpend(drop: SeriesDrop, decimals: number, digits = 0): string {
  return formatAmount(BigInt(drop.afterRaw) - BigInt(drop.beforeRaw), decimals, digits);
}

export function summaryEnd(summary: ChartSummary | undefined, at: number): number | null {
  if (!summary) return null;
  for (let index = summary.buckets.length - 1; index >= 0; index--) {
    const bucket = summary.buckets[index];
    if (!bucket || bucket.firstHour > at) continue;
    return bucket.lastHour >= at && summary.tiles && !summary.runs ? summary.tiles.valueAt(at) : bucket.lastValue;
  }
  return null;
}

export const rawOf = (value: number, decimals: number) => BigInt(Math.round(value * 10 ** decimals)).toString();
