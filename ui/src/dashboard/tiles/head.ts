import type { ChartSummary } from '../../charts/chartSummary';

type Line = Pick<ChartSummary, 'from' | 'buckets'>;

const followed = new Map<string, (() => number | null | undefined)[]>();

export const tileHead = (lane: string, tag: string) => `${lane}\u0000${tag}`;

export function followAtHead(head: string, read: () => number | null | undefined): () => void {
  let readers = followed.get(head);
  if (!readers) followed.set(head, readers = []);
  readers.push(read);
  return () => {
    const held = followed.get(head);
    const at = held?.lastIndexOf(read) ?? -1;
    if (at < 0) return;
    held!.splice(at, 1);
    if (!held!.length) followed.delete(head);
  };
}

export function valueAtHead(head: string, summary: Line, binHours: number, hour: number): number | null {
  const value = followed.get(head)?.at(-1)?.();
  if (value !== undefined) return value;
  const slot = Math.floor(hour / binHours) - Math.floor(summary.from / binHours) - 1;
  return slot >= 0 && slot < summary.buckets.length ? summary.buckets[slot]?.lastValue ?? null : null;
}
