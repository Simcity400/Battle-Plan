import { useMemo, useSyncExternalStore } from 'react';
import type { LostSpan } from '../charts/gaps';

/** The lost spans each product's prepared replies named, by `source\0branch\0product`: hours the
 * families it reads went on past, which no chart of its replies draws across. A chart of another
 * product is never cut at them. */
const spans = new Map<string, readonly LostSpan[]>();
const listeners = new Set<() => void>();
const NONE: readonly LostSpan[] = [];
let version = 0;

const keyOf = (sourceId: string, branchId: string, product: string) => `${sourceId}\u0000${branchId}\u0000${product}`;

const SERIES = new Set(['series-state', 'series-summary', 'series-next-change', 'events-page', 'events-clusters']);

/** What a prepared request replies for, as the service gates it: a series request its family
 * kind, any other its product. */
export function gapProduct(product: string, params: unknown): string | null {
  if (!SERIES.has(product)) return product;
  const family = params && typeof params === 'object' ? (params as Record<string, unknown>).family : undefined;
  const kind = family && typeof family === 'object' ? (family as Record<string, unknown>).kind : undefined;
  return typeof kind === 'string' ? kind : null;
}

/** The branch a prepared request names, where the service finds it. */
function branchOf(params: unknown): string | null {
  if (!params || typeof params !== 'object') return null;
  const record = params as Record<string, unknown>;
  const selection = record.selection as Record<string, unknown> | undefined;
  const branch = record.branchId ?? selection?.branchId;
  return typeof branch === 'string' ? branch : null;
}

/** Keeps the gaps among the `holes` a reply of `product` named, lost hours its families went on past. */
export function noteReplyGaps(sourceId: string, product: string, params: unknown, holes: unknown): void {
  if (!Array.isArray(holes)) return;
  const branch = branchOf(params);
  const replied = gapProduct(product, params);
  if (branch === null || replied === null) return;
  const found = holes.filter((hole): hole is LostSpan & { gap: true } => !!hole && typeof hole === 'object' && hole.gap === true
    && Number.isInteger(hole.firstHour) && Number.isInteger(hole.lastHour) && hole.firstHour <= hole.lastHour);
  if (!found.length) return;
  const key = keyOf(sourceId, branch, replied);
  const held = spans.get(key) ?? NONE;
  const merged = merge([...held, ...found.map(({ firstHour, lastHour }) => ({ firstHour, lastHour }))]);
  if (same(merged, held)) return;
  spans.set(key, merged);
  version += 1;
  for (const listener of listeners) listener();
}

const same = (a: readonly LostSpan[], b: readonly LostSpan[]) =>
  a.length === b.length && a.every((span, index) => span.firstHour === b[index].firstHour && span.lastHour === b[index].lastHour);

/** `found` in hour order, overlapping and adjacent spans joined. */
function merge(found: LostSpan[]): readonly LostSpan[] {
  found.sort((a, b) => a.firstHour - b.firstHour);
  const merged: LostSpan[] = [];
  for (const span of found) {
    const last = merged[merged.length - 1];
    if (last && span.firstHour <= last.lastHour + 1) merged[merged.length - 1] = { firstHour: last.firstHour, lastHour: Math.max(last.lastHour, span.lastHour) };
    else merged.push(span);
  }
  return merged;
}

/** Drops what was kept of a recording no longer open. */
export function forgetReplyGaps(sourceId: string): void {
  let changed = false;
  for (const key of [...spans.keys()]) if (key.startsWith(`${sourceId}\u0000`)) { spans.delete(key); changed = true; }
  if (!changed) return;
  version += 1;
  for (const listener of listeners) listener();
}

/** The lost spans the replies of `products` named on a recording's branch: one product's own,
 * or the union where one layer draws several. */
export function replyGaps(sourceId: string, branchId: string, products: readonly string[]): readonly LostSpan[] {
  if (products.length === 1) return spans.get(keyOf(sourceId, branchId, products[0])) ?? NONE;
  const found = products.flatMap(product => spans.get(keyOf(sourceId, branchId, product)) ?? []);
  return found.length ? merge(found) : NONE;
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

/** [`replyGaps`], kept current as replies name more; nothing without a recording's branch. */
export function useReplyGaps(sourceId: string | undefined, branchId: string | undefined, ...products: string[]): readonly LostSpan[] {
  const seen = useSyncExternalStore(subscribe, () => version, () => 0);
  const named = products.join('\u0000');
  return useMemo(() => {
    void seen;
    return sourceId && branchId && named ? replyGaps(sourceId, branchId, named.split('\u0000')) : NONE;
  }, [seen, sourceId, branchId, named]);
}
