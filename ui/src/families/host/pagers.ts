import type { SourceEntry, SourcePlan } from '../contract.ts';
import type { FamilyRowWindow } from './adapter.ts';
import { DEFAULT_TAIL_PAGE_SIZE, SnapshotPager, type PagerStep } from '../table/rowSource.ts';

export type PagedEntry = SourceEntry & { readonly read: Extract<SourceEntry['read'], { shape: 'paged-rows' }> };

interface Frozen {
  readonly identity: string;
  readonly hour: number;
}

export interface PagerSet {
  readonly sig: string;
  readonly pagers: Map<string, SnapshotPager>;
  readonly identities: Map<string, string>;
  readonly frozen: Map<string, Frozen>;
}

export function emptyPagers(sig: string): PagerSet {
  return { sig, pagers: new Map(), identities: new Map(), frozen: new Map() };
}

export function pagedEntries(plan: SourcePlan | null): PagedEntry[] {
  return (plan?.entries ?? []).filter((entry): entry is PagedEntry => entry.read.shape === 'paged-rows');
}

function identityOf(entry: PagedEntry, sig: string): string {
  const query = entry.read.tailQuery;
  const range = /\|range:-?\d+:-?\d+$/.exec(sig)?.[0] ?? '';
  return JSON.stringify([entry.name, entry.source.id, entry.source.revision, entry.branch, query.product, query.log ?? null, query.from, query.limit, range]);
}

export function replacePagers(previous: PagerSet, sig: string, entries: readonly PagedEntry[]): PagerSet {
  const next = emptyPagers(sig);
  for (const entry of entries) {
    const pager = previous.pagers.get(entry.name);
    const identity = previous.identities.get(entry.name);
    const carried = pager && pager.hour !== null && identity !== undefined ? { identity, hour: pager.hour } : previous.frozen.get(entry.name);
    if (carried && carried.identity === identityOf(entry, sig)) next.frozen.set(entry.name, carried);
  }
  return next;
}

function atHead(window: FamilyRowWindow, limit: number): boolean {
  const first = Math.max(0, Math.floor(window.dataFirst ?? window.first));
  const count = Math.max(0, Math.floor(window.dataCount ?? window.count));
  return count > 0 && first + count <= limit;
}

export function pagerWindow(held: PagerSet, entry: PagedEntry, window: FamilyRowWindow, installedHour: number | null): { readonly pager: SnapshotPager; readonly step: PagerStep } {
  let pager = held.pagers.get(entry.name);
  if (!pager) {
    pager = new SnapshotPager(entry.read.tailQuery.limit, entry.read.pageSize ?? DEFAULT_TAIL_PAGE_SIZE);
    held.pagers.set(entry.name, pager);
    held.identities.set(entry.name, identityOf(entry, held.sig));
  }
  const frozen = held.frozen.get(entry.name);
  const hour = frozen && installedHour !== null && Number.isFinite(installedHour) ? Math.min(frozen.hour, installedHour) : installedHour;
  const step = pager.window(window, hour);
  if (frozen && (pager.hour !== null || atHead(window, entry.read.tailQuery.limit))) held.frozen.delete(entry.name);
  return { pager, step };
}
