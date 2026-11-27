import type { ChartSummary } from '../../charts/chartSummary';

type Line = Pick<ChartSummary, 'from' | 'buckets'>;

export const tileHead = (lane: string, tag: string) => `${lane}\u0000${tag}`;

export function valueAtHead(_head: string, summary: Line, binHours: number, hour: number): number | null {
  const hourly = binHours === 1;
  const slot = Math.floor(hour / binHours) - Math.floor(summary.from / binHours) - (hourly ? 0 : 1);
  if (slot < 0 || slot >= summary.buckets.length) return null;
  const bucket = summary.buckets[slot];
  return (hourly ? bucket?.firstValue : bucket?.lastValue) ?? null;
}
