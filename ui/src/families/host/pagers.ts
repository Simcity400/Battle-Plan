import type { ArtResolutions } from '../art/provider.ts';
import { readPlanExact, type ExactReadOptions, type PagedSnapshotPage } from '../broker.ts';
import type { ArtProvider, BoundedReadView, SourceEntry, SourcePlan } from '../contract.ts';
import type { FamilyRowWindow } from './adapter.ts';
import { dataWindowOf, DEFAULT_TAIL_PAGE_SIZE, SnapshotPager, type PagedRows, type PagerStep } from '../table/rowSource.ts';

export type PagedSnapshot = Readonly<Record<string, Readonly<Record<number, PagedSnapshotPage>>>>;

export const NO_PAGED_SNAPSHOT: PagedSnapshot = Object.freeze({});

function samePage(a: PagedSnapshotPage | undefined, b: PagedSnapshotPage): boolean {
  if (!a || a.kind !== b.kind || a.hour !== b.hour || a.offset !== b.offset || a.limit !== b.limit) return false;
  return a.kind === 'answered' || (b.kind === 'refused' && a.reason === b.reason);
}

export function pagedSnapshots(
  pages: PagedRows | undefined,
  previous: PagedSnapshot = NO_PAGED_SNAPSHOT,
): PagedSnapshot {
  if (!pages) return Object.keys(previous).length === 0 ? previous : NO_PAGED_SNAPSHOT;
  const next: Record<string, Readonly<Record<number, PagedSnapshotPage>>> = {};
  let sameAll = true;
  for (const [name, sourcePages] of Object.entries(pages)) {
    const entries = Object.values(sourcePages);
    if (entries.length === 0) continue;
    const prevSource = previous[name];
    const mapped: Record<number, PagedSnapshotPage> = {};
    let sameSource = prevSource !== undefined && Object.keys(prevSource).length === entries.length;
    for (const page of entries) {
      const desc: PagedSnapshotPage = 'reply' in page
        ? { kind: 'answered', hour: page.hour, offset: page.offset, limit: page.limit }
        : { kind: 'refused', hour: page.hour, offset: page.offset, limit: page.limit, reason: page.refused };
      mapped[page.offset] = desc;
      if (sameSource && !samePage(prevSource?.[page.offset], desc)) sameSource = false;
    }
    next[name] = sameSource ? prevSource! : mapped;
    if (!sameSource) sameAll = false;
  }
  const names = Object.keys(next);
  if (names.length === 0) return Object.keys(previous).length === 0 ? previous : NO_PAGED_SNAPSHOT;
  if (sameAll && Object.keys(previous).length === names.length) return previous;
  return next;
}

export function createExactReadView(
  plan: SourcePlan,
  snapshots: PagedSnapshot,
  options: Omit<ExactReadOptions, 'snapshotHour' | 'pagedSnapshot' | 'rowWindow' | 'art'> & {
    readonly art?: (resolutions: ArtResolutions | undefined) => ArtProvider;
  },
): (wantedHour: number, rowWindow: FamilyRowWindow | null, artResolutions?: ArtResolutions) => Promise<BoundedReadView | null> {
  return (wantedHour, rowWindow, artResolutions) => readPlanExact(plan, wantedHour, {
    ...options,
    art: options.art?.(artResolutions),
    pagedSnapshot: name => snapshots[name] ?? null,
    rowWindow,
  });
}

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
  const data = dataWindowOf(window);
  return data !== null && data.first + data.count <= limit;
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
  if (frozen && atHead(window, entry.read.tailQuery.limit)) held.frozen.delete(entry.name);
  return { pager, step };
}
