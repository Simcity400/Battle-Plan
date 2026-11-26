import type { SourceEntry } from '../contract.ts';
import { DEFAULT_TAIL_PAGE_SIZE, SnapshotPager } from '../table/rowSource.ts';

export interface PagerSet {
  readonly sig: string;
  readonly pagers: Map<string, SnapshotPager>;
  readonly identities: Map<string, string>;
  readonly frozen: Map<string, number>;
}

export function emptyPagers(sig: string): PagerSet {
  return { sig, pagers: new Map(), identities: new Map(), frozen: new Map() };
}

export function replacePagers(previous: PagerSet, sig: string): PagerSet {
  const next = emptyPagers(sig);
  for (const [identity, hour] of previous.frozen) next.frozen.set(identity, hour);
  for (const [name, pager] of previous.pagers) {
    const identity = previous.identities.get(name);
    if (identity === undefined) continue;
    if (pager.hour === null) next.frozen.delete(identity);
    else next.frozen.set(identity, pager.hour);
  }
  return next;
}

export function pagerFor(held: PagerSet, entry: SourceEntry & { readonly read: Extract<SourceEntry['read'], { shape: 'paged-rows' }> }): { readonly pager: SnapshotPager; readonly frozenAt: number | null } {
  const found = held.pagers.get(entry.name);
  if (found) return { pager: found, frozenAt: null };
  const identity = JSON.stringify([entry.name, entry.source.id, entry.source.revision, entry.branch]);
  const pager = new SnapshotPager(entry.read.tailQuery.limit, entry.read.pageSize ?? DEFAULT_TAIL_PAGE_SIZE);
  held.pagers.set(entry.name, pager);
  held.identities.set(entry.name, identity);
  const frozenAt = held.frozen.get(identity) ?? null;
  held.frozen.delete(identity);
  return { pager, frozenAt };
}
