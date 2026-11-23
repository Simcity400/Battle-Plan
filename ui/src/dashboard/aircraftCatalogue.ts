import { asFailure } from './queryError.ts';
import { useCallback, useEffect, useRef, useState } from 'react';
import type { AircraftCatalogueParams, AircraftCatalogueReply, AircraftEntry, AircraftOrigin, AircraftOriginCount, AircraftOriginFilter, AircraftRefusalCount, AircraftRefusalKind } from '../generated/dashboard.ts';
import type { DashboardClient, DashboardSource } from './client';
import { useContentRevision } from './contentRevision.ts';
import { loadTransientProduct } from './products.ts';
import { busyWait } from '../shell/busy.ts';
import { productRequest } from './request.ts';

export const AIRCRAFT_PAGE = 100;
export const AIRCRAFT_QUERY_DELAY = 150;

export function pagesFor(first: number, last: number, size = AIRCRAFT_PAGE): number[] {
  if (last < first || last < 0) return [];
  const pages: number[] = [];
  for (let page = Math.floor(Math.max(0, first) / size); page <= Math.floor(last / size); page++) pages.push(page);
  return pages;
}

export class AircraftPages {
  readonly identity: string;
  readonly size: number;
  matching: number | null = null;
  found: number | null = null;
  refusals: AircraftRefusalCount[] = [];
  origins: AircraftOriginCount[] = [];
  warnings: string[] = [];
  private readonly pages = new Map<number, AircraftEntry[]>();
  private readonly pending = new Set<number>();
  private readonly failed = new Map<number, Error>();

  constructor(identity: string, size = AIRCRAFT_PAGE) {
    this.identity = identity;
    this.size = size;
  }

  entry(index: number): AircraftEntry | undefined {
    return this.pages.get(Math.floor(index / this.size))?.[index % this.size];
  }

  wants(page: number): boolean {
    if (this.pages.has(page) || this.pending.has(page) || this.failed.has(page)) return false;
    return this.matching === null ? page === 0 : page * this.size < this.matching;
  }

  begin(page: number): void { this.pending.add(page); this.failed.delete(page); }

  cancel(page: number): void { this.pending.delete(page); }

  accept(page: number, reply: AircraftCatalogueReply): boolean {
    this.pending.delete(page);
    if (reply.first !== page * this.size) {
      this.failed.set(page, new Error(`The catalogue answered rows from ${reply.first} for rows from ${page * this.size}`));
      return false;
    }
    this.pages.set(page, reply.entries);
    this.matching = reply.matching;
    this.found = reply.found;
    this.refusals = reply.refusals;
    this.origins = reply.origins;
    this.warnings = reply.warnings;
    return true;
  }

  fail(page: number, message: Error): void { this.pending.delete(page); this.failed.set(page, message); }
  retry(page: number): void { this.failed.delete(page); }
  error(page: number): Error | undefined { return this.failed.get(page); }
  firstError(): Error | undefined { return this.failed.values().next().value; }
  loading(): boolean { return this.pending.size > 0; }

  flyable(): number | null {
    return this.matching === null ? null : this.matching - this.refusals.reduce((sum, refusal) => sum + refusal.count, 0);
  }
}

export function originFilter(origin: AircraftOrigin): AircraftOriginFilter {
  return origin.kind === 'game' ? 'game' : { mod: origin.id };
}

export function sameOrigin(a: AircraftOriginFilter | null, b: AircraftOriginFilter | null): boolean {
  if (a === null || b === null || a === 'game' || b === 'game') return a === b;
  return a.mod === b.mod;
}

export function originLabel(origin: AircraftOrigin): string {
  return origin.kind === 'game' ? 'Vanilla' : origin.name;
}

const TAG = /^[A-Z0-9]{3}$/;

export function countryTag(entry: Pick<AircraftEntry, 'references'>): string | null {
  for (const reference of entry.references) if (reference.kind === 'ladder' && reference.prefix && TAG.test(reference.prefix)) return reference.prefix;
  for (const reference of entry.references) if (reference.kind === 'pool' && TAG.test(reference.bucket)) return reference.bucket;
  return null;
}

export function aircraftRowInteractive(pages: AircraftPages, index: number): boolean {
  if (pages.error(Math.floor(index / pages.size)) !== undefined) return true;
  const entry = pages.entry(index);
  return !!entry && !entry.refusal;
}

const unreadable = (kind: AircraftRefusalKind) => kind === 'declaration';

export function refusalText(kind: AircraftRefusalKind): string {
  return unreadable(kind) ? 'Can’t be flown: BattlePlan can’t read this model.' : 'Can’t be flown: its 3D model is missing.';
}

export function refusalLine(refusals: readonly AircraftRefusalCount[]): { text: string; tooltip: string; detail: string } | null {
  const counted = refusals.filter(refusal => refusal.count > 0);
  const total = counted.reduce((sum, refusal) => sum + refusal.count, 0);
  if (!total) return null;
  const missing = counted.filter(refusal => !unreadable(refusal.kind)).reduce((sum, refusal) => sum + refusal.count, 0);
  const tooltip = [missing ? `${missing.toLocaleString()} with a missing 3D model` : '',
    total - missing ? `${(total - missing).toLocaleString()} that BattlePlan can’t read` : ''].filter(Boolean).join('\n');
  return { text: `${total.toLocaleString()} ${total === 1 ? 'model' : 'models'} can’t be flown`, tooltip,
    detail: counted.map(refusal => `${refusal.count.toLocaleString()}: ${refusal.label}`).join('\n') };
}

export const CATALOGUE_NOTE = 'Some game files couldn’t be read, so this list may be incomplete.';

export function catalogueParams(query: string, first: number, count: number, origin: AircraftOriginFilter | null): AircraftCatalogueParams {
  return { query: query.trim(), first, count, origin };
}

export interface AircraftCatalogueState {
  pages: AircraftPages | null;
  version: number;
  request: (first: number, last: number) => void;
  retry: (index: number) => void;
}

export function useAircraftCatalogue(client: DashboardClient, source: DashboardSource | null, origin: AircraftOriginFilter | null, query: string, active: boolean): AircraftCatalogueState {
  const content = useContentRevision(client);
  const [asked, setAsked] = useState(query);
  useEffect(() => {
    if (asked === query) return;
    return busyWait(() => setAsked(query), AIRCRAFT_QUERY_DELAY);
  }, [query, asked]);
  const identity = source && active ? JSON.stringify([source.id, source.revision, content, origin, asked.trim()]) : null;
  const [version, setVersion] = useState(0);
  const current = useRef<{ pages: AircraftPages; controllers: Map<AbortController, number> } | null>(null);
  if (identity && current.current?.pages.identity !== identity) {
    for (const controller of current.current?.controllers.keys() ?? []) controller.abort();
    current.current = { pages: new AircraftPages(identity), controllers: new Map() };
  } else if (!identity && current.current) {
    for (const controller of current.current.controllers.keys()) controller.abort();
    current.current = null;
  }
  const held = current.current;
  useEffect(() => () => {
    if (!held) return;
    for (const [controller, page] of held.controllers) { controller.abort(); held.pages.cancel(page); }
    held.controllers.clear();
  }, [held]);
  const load = useCallback((page: number) => {
    const state = current.current;
    if (!state || !source || !state.pages.wants(page)) return;
    const { pages, controllers } = state;
    pages.begin(page);
    const controller = new AbortController();
    controllers.set(controller, page);
    const params = catalogueParams(asked, page * pages.size, pages.size, origin);
    void loadTransientProduct<AircraftCatalogueReply>(client, productRequest({ sourceId: source.id, revision: source.revision, product: 'aircraft-catalogue', params }), controller.signal, source)
      .then(reply => { if (!controller.signal.aborted) pages.accept(page, reply); },
        error => { if (!controller.signal.aborted) pages.fail(page, asFailure(error)); })
      .finally(() => {
        controllers.delete(controller);
        if (!controller.signal.aborted && current.current?.pages === pages) setVersion(version => version + 1);
      });
  }, [client, source, origin, asked]);
  useEffect(() => { if (identity) load(0); }, [identity, load]);
  const request = useCallback((first: number, last: number) => {
    for (const page of pagesFor(first, last + (current.current?.pages.size ?? AIRCRAFT_PAGE))) load(page);
  }, [load]);
  const retry = useCallback((index: number) => {
    const pages = current.current?.pages;
    if (!pages) return;
    const page = Math.floor(index / pages.size);
    pages.retry(page);
    load(page);
  }, [load]);
  return { pages: held?.pages ?? null, version, request, retry };
}
