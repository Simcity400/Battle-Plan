import { asFailure } from './queryError.ts';
import { useCallback, useEffect, useRef, useState } from 'react';
import type { AircraftCatalogueParams, AircraftCatalogueReply, AircraftEntry, AircraftReference, AircraftRefusalCount, AircraftScope } from '../generated/dashboard.ts';
import type { DashboardClient, DashboardSource } from './client';
import { useContentRevision } from './contentRevision.ts';
import { loadTransientProduct } from './products.ts';
import { busyWait } from '../shell/busy.ts';

/** Entries one catalogue request answers. Every entry stays reachable a page at a time. */
export const AIRCRAFT_PAGE = 100;
/** How long a query stays unchanged before it is asked. */
export const AIRCRAFT_QUERY_DELAY = 150;

/** The pages that hold rows `first` through `last`, inclusive. */
export function pagesFor(first: number, last: number, size = AIRCRAFT_PAGE): number[] {
  if (last < first || last < 0) return [];
  const pages: number[] = [];
  for (let page = Math.floor(Math.max(0, first) / size); page <= Math.floor(last / size); page++) pages.push(page);
  return pages;
}

/** The pages of one listing (a source, content, scope and query), as they arrive. Counts come with the first page answered. */
export class AircraftPages {
  readonly identity: string;
  readonly size: number;
  matching: number | null = null;
  aircraft: number | null = null;
  all: number | null = null;
  refusals: AircraftRefusalCount[] = [];
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

  /** Whether `page` should be asked for: neither held, asked nor failed, and inside the listing once its length is known. */
  wants(page: number): boolean {
    if (this.pages.has(page) || this.pending.has(page) || this.failed.has(page)) return false;
    return this.matching === null ? page === 0 : page * this.size < this.matching;
  }

  begin(page: number): void { this.pending.add(page); this.failed.delete(page); }

  /** Forgets a cancelled request, so the page is asked again. */
  cancel(page: number): void { this.pending.delete(page); }

  /** Takes an answered page; a reply for other rows than the page's is refused. */
  accept(page: number, reply: AircraftCatalogueReply): boolean {
    this.pending.delete(page);
    if (reply.first !== page * this.size) {
      this.failed.set(page, new Error(`The catalogue answered rows from ${reply.first} for rows from ${page * this.size}`));
      return false;
    }
    this.pages.set(page, reply.entries);
    this.matching = reply.matching;
    this.aircraft = reply.aircraft;
    this.all = reply.all;
    this.refusals = reply.refusals;
    this.warnings = reply.warnings;
    return true;
  }

  fail(page: number, message: Error): void { this.pending.delete(page); this.failed.set(page, message); }
  /** Forgets a failed page so it is asked again. */
  retry(page: number): void { this.failed.delete(page); }
  error(page: number): Error | undefined { return this.failed.get(page); }
  /** The first failure, for the listing's own notice. */
  firstError(): Error | undefined { return this.failed.values().next().value; }
  loading(): boolean { return this.pending.size > 0; }
}

/** Whether row `index` is a keyboard destination: a flyable entry, or a failed page's retry.
 * Refused entries keep their place in the list but are not destinations, nor are rows whose
 * page has not arrived. */
export function aircraftRowInteractive(pages: AircraftPages, index: number): boolean {
  if (pages.error(Math.floor(index / pages.size)) !== undefined) return true;
  const entry = pages.entry(index);
  return !!entry && !entry.refusal;
}

/** What one reference says, in a sentence fragment. */
export function referenceText(reference: AircraftReference): string {
  switch (reference.kind) {
    case 'pool': return `${reference.bucket} pool for ${reference.equipment}`;
    case 'ladder': return `${reference.prefix ? `${reference.prefix} ` : ''}${reference.key} model`;
    case 'design': return `saved model of a ${reference.equipment} design`;
    case 'cloneOf': return `clone of ${reference.of}`;
    case 'clonedBy': return `cloned by ${reference.by}`;
  }
}

/** The counted refusals as one line, largest first in the server's order. */
export function refusalLine(refusals: readonly AircraftRefusalCount[]): string | null {
  if (!refusals.length) return null;
  return `Not flyable: ${refusals.map(refusal => `${refusal.count.toLocaleString()} ${refusal.label}`).join(', ')}`;
}

export interface AircraftCatalogueState {
  pages: AircraftPages | null;
  /** Grows each time a page arrives or fails, as the pages change in place. */
  version: number;
  /** Asks for the pages that hold rows `first` through `last`. */
  request: (first: number, last: number) => void;
  /** Asks again for a page that failed. */
  retry: (index: number) => void;
}

/** One scope of the recording's aircraft catalogue for a query, a page at a time: only the pages rows ask for are requested, and a change of source, content, scope or query drops every page and cancels what is in flight. */
export function useAircraftCatalogue(client: DashboardClient, source: DashboardSource | null, scope: AircraftScope, query: string, active: boolean): AircraftCatalogueState {
  const content = useContentRevision(client);
  const [asked, setAsked] = useState(query);
  useEffect(() => {
    if (asked === query) return;
    return busyWait(() => setAsked(query), AIRCRAFT_QUERY_DELAY);
  }, [query, asked]);
  const identity = source && active ? JSON.stringify([source.id, source.revision, content, scope, asked.trim()]) : null;
  const [version, setVersion] = useState(0);
  // Each request in flight, with its page.
  const current = useRef<{ pages: AircraftPages; controllers: Map<AbortController, number> } | null>(null);
  if (identity && current.current?.pages.identity !== identity) {
    for (const controller of current.current?.controllers.keys() ?? []) controller.abort();
    current.current = { pages: new AircraftPages(identity), controllers: new Map() };
  } else if (!identity && current.current) {
    for (const controller of current.current.controllers.keys()) controller.abort();
    current.current = null;
  }
  const held = current.current;
  // A listing kept through an effect's replay (Fast Refresh) asks again for what it cancelled.
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
    const params: AircraftCatalogueParams = { scope, query: asked.trim(), first: page * pages.size, count: pages.size };
    void loadTransientProduct<AircraftCatalogueReply>(client, { sourceId: source.id, revision: source.revision, product: 'aircraft-catalogue', params }, controller.signal, source)
      .then(reply => { if (!controller.signal.aborted) pages.accept(page, reply); },
        error => { if (!controller.signal.aborted) pages.fail(page, asFailure(error)); })
      .finally(() => {
        controllers.delete(controller);
        if (!controller.signal.aborted && current.current?.pages === pages) setVersion(version => version + 1);
      });
  }, [client, source, scope, asked]);
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
