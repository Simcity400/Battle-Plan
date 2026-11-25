import type { ChartSummary } from '../../charts/chartSummary';

type Line = Pick<ChartSummary, 'from' | 'buckets'>;

type Followed = { readonly hour: number; readonly value: number | null | undefined };

const followed = new Map<string, (() => Followed)[]>();

export const tileHead = (lane: string, tag: string) => `${lane}\u0000${tag}`;

export function followAtHead(head: string, read: () => Followed): () => void {
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
  const held = followed.get(head)?.at(-1)?.();
  if (held && held.value !== undefined && Math.floor(hour) >= held.hour) return held.value;
  const hourly = binHours === 1;
  const slot = Math.floor(hour / binHours) - Math.floor(summary.from / binHours) - (hourly ? 0 : 1);
  if (slot < 0 || slot >= summary.buckets.length) return null;
  const bucket = summary.buckets[slot];
  return (hourly ? bucket?.firstValue : bucket?.lastValue) ?? null;
}
