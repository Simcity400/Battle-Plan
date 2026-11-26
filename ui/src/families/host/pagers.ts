import type { SourceEntry } from '../contract.ts';
import type { FamilyRowWindow } from './adapter.ts';
import { DEFAULT_TAIL_PAGE_SIZE, SnapshotPager, type PagerStep, type SnapshotPages } from '../table/rowSource.ts';

export type PagedEntry = SourceEntry & { readonly read: Extract<SourceEntry['read'], { shape: 'paged-rows' }> };

interface Frozen {
  readonly identity: string;
  readonly hour: number;
  readonly pager: SnapshotPager;
}

export interface PagerSet {
  readonly sig: string;
  readonly pagers: Map<string, SnapshotPager>;
  readonly identities: Map<string, string>;
  readonly frozen: Map<string, Frozen>;
  readonly shown: Map<string, SnapshotPager>;
}

export function emptyPagers(sig: string): PagerSet {
  return { sig, pagers: new Map(), identities: new Map(), frozen: new Map(), shown: new Map() };
}

function identityOf(entry: PagedEntry): string {
  const query = entry.read.tailQuery;
  return JSON.stringify([entry.name, entry.source.id, entry.source.revision, entry.branch, query.product, query.log ?? null, query.from, query.limit]);
}

export function replacePagers(previous: PagerSet, sig: string): PagerSet {
  const next = emptyPagers(sig);
  for (const [name, frozen] of previous.frozen) {
    if (!previous.pagers.has(name)) next.frozen.set(name, frozen);
  }
  for (const [name, pager] of previous.pagers) {
    const identity = previous.identities.get(name);
    if (identity !== undefined && pager.hour !== null) next.frozen.set(name, { identity, hour: pager.hour, pager });
  }
  return next;
}

function atHead(window: FamilyRowWindow, limit: number): boolean {
  const first = Math.max(0, Math.floor(window.dataFirst ?? window.first));
  const count = Math.max(0, Math.floor(window.dataCount ?? window.count));
  return first + count <= limit;
}

export function pagerWindow(held: PagerSet, entry: PagedEntry, window: FamilyRowWindow, installedHour: number | null): { readonly pager: SnapshotPager; readonly step: PagerStep } {
  const identity = identityOf(entry);
  let pager = held.pagers.get(entry.name);
  if (!pager) {
    pager = new SnapshotPager(entry.read.tailQuery.limit, entry.read.pageSize ?? DEFAULT_TAIL_PAGE_SIZE);
    held.pagers.set(entry.name, pager);
    held.identities.set(entry.name, identity);
  }
  const frozen = held.frozen.get(entry.name);
  const inherited = frozen && frozen.identity === identity ? frozen : null;
  const hour = inherited && installedHour !== null && Number.isFinite(installedHour) ? Math.min(inherited.hour, installedHour) : installedHour;
  const step = pager.window(window, hour);
  if (frozen && (!inherited || pager.hour !== null || atHead(window, entry.read.tailQuery.limit))) held.frozen.delete(entry.name);
  if (inherited && pager.hour === inherited.hour) held.shown.set(entry.name, inherited.pager);
  return { pager, step };
}

export function pagesOf(held: PagerSet, name: string): SnapshotPages | undefined {
  const pager = held.pagers.get(name);
  if (!pager) return undefined;
  const shown = held.shown.get(name);
  if (!shown) return pager.pages();
  if (pager.shownHour !== null || pager.hour === null) {
    held.shown.delete(name);
    return pager.pages();
  }
  return shown.pages();
}
