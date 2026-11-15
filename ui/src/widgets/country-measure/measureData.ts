import type { SeriesDrop } from '../../generated/dashboard';
import { formatAmount } from '../../charts/valueAxis.ts';
import { shownClusters } from '../../charts/markSummary.ts';
import type { MarkCluster, MarkSummary } from '../../charts/markSummary';
import type { ChartSummary } from '../../charts/chartSummary';

/** A balance in whole numbers unless `digits` asks for decimals; the raw stays exact. */
export const formatBalance = (raw: string, decimals: number, digits = 0) => formatAmount(raw, decimals, digits);

/** The balance changes a list opens on: the hovered cluster's, or from the first recorded change through the last
 * the head has reached (`until`); every change when nothing is drawn. Ordinals, `end` past the last. */
export function changesRange(marks: MarkSummary | null | undefined, hovered: MarkCluster | null, until: number | null): { first: string; end?: string } {
  const first = hovered?.firstOrdinal ?? marks?.clusters[0]?.firstOrdinal ?? '0';
  // Tiles hold clusters past the head: the last is the last shown, cut to the bins the head has passed.
  const last = hovered ?? (marks ? shownClusters(marks, until).at(-1) : undefined);
  return { first, end: last ? (BigInt(last.firstOrdinal) + BigInt(last.count)).toString() : undefined };
}

/** A drop's net change, negative, written to `digits`. */
export function netSpend(drop: SeriesDrop, decimals: number, digits = 0): string {
  return formatAmount(BigInt(drop.afterRaw) - BigInt(drop.beforeRaw), decimals, digits);
}

/** The value a line is drawn at where it ends at or before `at`: the head bin's at `at` as the painter draws it
 * (the followed value at the head), else the last value of the last bin before it. */
export function summaryEnd(summary: ChartSummary | undefined, at: number): number | null {
  if (!summary) return null;
  for (let index = summary.buckets.length - 1; index >= 0; index--) {
    const bucket = summary.buckets[index];
    if (!bucket || bucket.firstHour > at) continue;
    return bucket.lastHour >= at && summary.tiles && !summary.runs ? summary.tiles.valueAt(at) : bucket.lastValue;
  }
  return null;
}

/** A value of `decimals` places as the raw fixed-point text the balance formatter takes. */
export const rawOf = (value: number, decimals: number) => BigInt(Math.round(value * 10 ** decimals)).toString();
