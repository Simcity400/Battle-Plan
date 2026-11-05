import { type Failure, asFailure } from './queryError.ts';
import { useEffect, useState } from 'react';
import type { DashboardClient, DashboardSource, ProductReply, ProductRequest } from './client';
import { validateProductReply } from './client.ts';
import { ResourceCache } from './resourceCache.ts';
import { forgetFlags } from './countryFlags.ts';
import { forgetFacts } from './sharedFacts.ts';
import { forgetTimelines } from './countryTimeline.ts';
import { forgetReplySizes } from './replySizes.ts';
import { contentRevision, useContentRevision } from './contentRevision.ts';
import { isPreparedProduct, preparedSeriesQuery } from './preparedSeries.ts';
import { validateCountryGroupReply } from './groupsReply.ts';
import { pageJoin } from './pageJoins.ts';
import { CONTENT_PRODUCTS } from '../generated/dashboard.ts';
import { newId } from '../shell/ids.ts';
import { finalPreparedReply } from './preparedPrefix.ts';
import { useHeldFailure, useHeldFailures } from './heldFailures.ts';
import { useViewRead } from './viewRead.ts';
import { inFrame } from './frameUpdates.ts';
import { perfLog } from '../shell/perf.ts';

/** Each reply's length as the transport received it, so the cache sizes a reply
 * without serialising it again; a reply without one is measured by serialising. */
export const replyLengths = new WeakMap<object, number>();

/** The length `reply` was received at: its own, or its data's for a prepared reply, which is
 * kept wrapped anew around the data the transport received. */
function receivedLength(reply: ProductReply<unknown>): number | undefined {
  const data: unknown = reply.data;
  return replyLengths.get(reply) ?? (data !== null && typeof data === 'object' ? replyLengths.get(data) : undefined);
}

/** Two bounded caches of final product replies per client. One-shot requests share native
 * work; progressive replies and polling belong to each mount. Replies that follow the playhead
 * (`hours`) are kept apart, so however many hours are read, they never push out the replies that
 * do not follow it, which are the costliest to read again. */
const caches = new WeakMap<DashboardClient, { shared: ResourceCache<ProductReply<unknown>>; hours: ResourceCache<ProductReply<unknown>> }>();
const replySize = (reply: ProductReply<unknown>) => (receivedLength(reply) ?? JSON.stringify(reply).length) * 2;
function cache(client: DashboardClient, hours = false): ResourceCache<ProductReply<unknown>> {
  let entry = caches.get(client);
  if (!entry) {
    entry = { shared: new ResourceCache(128, 64 * 1024 * 1024, replySize), hours: new ResourceCache(64, 32 * 1024 * 1024, replySize) };
    caches.set(client, entry);
  }
  return hours ? entry.hours : entry.shared;
}

const contentProducts: ReadonlySet<string> = new Set(CONTENT_PRODUCTS);

export function productKey(request: ProductRequest, content = 0, prepared = false): string {
  const key: unknown[] = [request.sourceId, request.revision, request.product, request.params];
  const contentProduct = request.product === 'ships-sunk-manifest' ? 'ships-sunk' : request.product;
  if (contentProducts.has(contentProduct) || prepared || (request.params as { family?: { kind?: string } } | null)?.family?.kind === 'naval-losses') key.push(content);
  if (prepared) key.push('prepared');
  return JSON.stringify(key);
}

function usesPrepared(client: DashboardClient, request: ProductRequest, source?: DashboardSource): boolean {
  const prepared = isPreparedProduct(request.product) || (!!client.preparedQuery && (request.product === 'politics-ledger' || request.product === 'politics-state-history' || request.product === 'civil-wars')) || (!!source && !!client.preparedQuery && (request.product === 'army-size' || request.product === 'supply-health' || request.product === 'series-summary' || request.product === 'military-air-ledger' || request.product === 'air-aces' || request.product === 'air-ace-portraits' || request.product === 'raids' || request.product === 'air-superiority' || request.product === 'casualty-pairs' || request.product === 'equipment-losses' || request.product === 'subjects' || request.product === 'wars' || request.product === 'country-groups' || request.product === 'party-history' || request.product === 'scoreboard' || request.product === 'series-state' || request.product === 'unit-leaders' || request.product === 'construction-log' || request.product === 'construction-log-manifest' || request.product === 'construction-spending' || request.product === 'resource-balance' || request.product === 'trade-flows' || request.product === 'production-spending' || request.product === 'dockyard-spending' || request.product === 'military-factory-spending' || request.product === 'casualty-breakdown' || request.product === 'series-next-change' || request.product === 'events-page' || request.product === 'events-clusters' || request.product === 'occupied-states' || request.product === 'occupied-states-manifest' || request.product === 'land-owners' || request.product === 'espionage-log' || request.product === 'operative-placements' || request.product === 'operatives-roster' || request.product === 'agency-comparison' || request.product === 'decryption'));
  if (prepared && source && (source.id !== request.sourceId || source.revision !== request.revision)) throw new Error('The prepared product source does not match its request.');
  return prepared;
}

interface ProgressiveListener { publish: (value: unknown) => void; pending?: () => void }
/** A mounted prepared read in flight, which every mount asking the same thing joins. */
interface ProgressiveRead {
  users: number;
  /** The newest reply published, for a mount joining late. */
  latest?: { value: unknown };
  /** Whether it waits for preparation to cover it. */
  waiting: boolean;
  listeners: Set<ProgressiveListener>;
  controller: AbortController;
  done: Promise<unknown>;
}
const progressive = new WeakMap<DashboardClient, Map<string, ProgressiveRead>>();

/** What each validator made of a reply's data, or why it refused: caches hold raw replies, and each
 * consumer is given the data its own validator makes, never another's, validated once. */
const validations = new WeakMap<object, WeakMap<(data: unknown) => unknown, { data: unknown } | { refused: unknown }>>();
function validatedData<T>(data: unknown, validate?: (data: unknown) => T): T {
  if (!validate) return data as T;
  if (data === null || typeof data !== 'object') return validate(data);
  let made = validations.get(data);
  // Held weakly: a validator made afresh each render is let go with what it made.
  if (!made) validations.set(data, made = new WeakMap());
  let result = made.get(validate);
  if (!result) {
    try { result = { data: validate(data) }; } catch (refused) { result = { refused }; }
    made.set(validate, result);
  }
  if ('refused' in result) throw result.refused;
  return result.data as T;
}

/** `promise`, or a cancellation as soon as `signal` aborts. */
function untilAborted<V>(promise: Promise<V>, signal: AbortSignal): Promise<V> {
  return new Promise<V>((resolve, reject) => {
    const abort = () => reject(new DOMException('Request cancelled', 'AbortError'));
    signal.addEventListener('abort', abort, { once: true });
    promise.then(value => { signal.removeEventListener('abort', abort); resolve(value); },
      error => { signal.removeEventListener('abort', abort); reject(error); });
    if (signal.aborted) abort();
  });
}

function pagedKind(request: ProductRequest): string | null {
  if (request.product === 'resource-balance' || request.product === 'trade-flows') return request.product;
  // Only leader choices are paged; every other unit-leaders view is one reply.
  if (request.product === 'unit-leaders') return (request.params as { view?: unknown } | null)?.view === 'choices' ? 'unit-leaders-choices' : null;
  if (request.product !== 'series-summary') return null;
  const kind = (request.params as { family?: { kind?: unknown } } | null)?.family?.kind;
  return typeof kind === 'string' ? kind : null;
}

/** One-shot loads return their first replies. Mounted prepared loads publish each
 * accepted extent; only final replies enter a bounded cache, the hours' own for a read that
 * follows the playhead (`hours`). Mounts asking the same thing share one read, each told every
 * extent and, while it waits for preparation to cover it, `pending`; a one-shot load may join
 * another's request and is told nothing. A paged reply is read page after page, each published
 * joined to the pages before it. */
export async function loadProduct<T>(client: DashboardClient, request: ProductRequest, signal: AbortSignal, validate?: (data: unknown) => T, source?: DashboardSource, restart = false, publish?: (data: T) => void, pending?: () => void, hours = false): Promise<T> {
  const kind = pagedKind(request);
  if (kind === null) return loadPage(client, request, signal, validate, source, restart, publish, pending, hours);
  const join = pageJoin(kind);
  // Each page is validated alone as it is cached; what the consumer is given is the join.
  const page = (data: unknown) => { validatedData(data, validate); return data; };
  const pages: unknown[] = [];
  const joined = () => validatedData(pages.length === 1 ? pages[0] : join!(pages), validate);
  let asked = request;
  for (;;) {
    const at = pages.length;
    const tell = publish && ((data: unknown) => { if (signal.aborted) return; pages[at] = data; pages.length = at + 1; publish(joined()); });
    const data = await loadPage<unknown>(client, asked, signal, page, source, restart, tell, pending, hours);
    pages[at] = data; pages.length = at + 1;
    const cursor = data !== null && typeof data === 'object' ? (data as { cursor?: unknown }).cursor : undefined;
    if (typeof cursor !== 'string') return joined();
    if (!join) throw new Error('This reply arrives in pages that this view cannot join.');
    if (cursor === (asked.params as { cursor?: unknown } | null)?.cursor) throw new Error('The paged reply named the page it answered as its next.');
    asked = { ...request, params: { ...(request.params as object), cursor } };
  }
}

async function loadPage<T>(client: DashboardClient, request: ProductRequest, signal: AbortSignal, validate?: (data: unknown) => T, source?: DashboardSource, restart = false, publish?: (data: T) => void, pending?: () => void, hours = false): Promise<T> {
  const prepared = usesPrepared(client, request, source);
  const replies = cache(client, hours), key = productKey(request, contentRevision(client).snapshot(), prepared);
  // A final reply either cache holds answers at once, whichever kind of read settled it.
  const settled = () => replies.getSettled(key) ?? cache(client, !hours).getSettled(key);
  const told = (data: unknown) => validatedData(data, validate);
  if (prepared && publish) {
    if (signal.aborted) throw new DOMException('Request cancelled', 'AbortError');
    const cached = settled();
    if (cached) {
      perfLog?.cached(request, 'hit');
      const reply = await untilAborted(cached, signal);
      // An abort after the reply settled but before this continuation still cancels.
      if (signal.aborted) throw new DOMException('Request cancelled', 'AbortError');
      const data = told(reply.data);
      publish(data);
      return data;
    }
    // Mounts asking for what another is reading join its read, whichever cache it lands in: each
    // is told every reply it publishes, validated its own way. A retry reads afresh. No partial
    // reply is cached.
    const reads = progressive.get(client) ?? new Map<string, ProgressiveRead>();
    progressive.set(client, reads);
    let read = restart ? undefined : reads.get(key);
    if (read) perfLog?.cached(request, 'joined');
    if (!read) {
      const created: ProgressiveRead = { users: 0, waiting: false, listeners: new Set(), controller: new AbortController(), done: Promise.resolve(undefined) };
      // A unique in-flight entry keeps source release cancellation in the same
      // cache machinery. It retains no result; the final reply uses the real key.
      created.done = replies.get(JSON.stringify([request.sourceId, key, newId()]), async shared => {
        const data = await preparedSeriesQuery(client, source ?? { id: request.sourceId, revision: request.revision } as DashboardSource,
          request.product as Parameters<typeof preparedSeriesQuery>[2], request.params, shared, restart, () => {
            created.waiting = true;
            for (const listener of created.listeners) listener.pending?.();
          }, undefined, value => {
            created.latest = { value };
            for (const listener of created.listeners) listener.publish(value);
          });
        if (shared.aborted) throw new DOMException('Request cancelled', 'AbortError');
        // A final reply the first mount's validation refuses stays uncached; the others still take it.
        if (finalPreparedReply(data)) {
          let accepted = true;
          try { told(data); } catch { accepted = false; }
          if (accepted) replies.put(key, { ...request, data });
        }
        return { ...request, data };
      }, created.controller.signal, () => false).then(reply => reply.data).finally(() => { if (reads.get(key) === created) reads.delete(key); });
      if (!restart) reads.set(key, created);
      read = created;
    }
    const joined = read;
    // What this mount is told failing fails this mount alone.
    let fail!: (error: unknown) => void;
    const failed = new Promise<never>((_, reject) => { fail = reject; });
    const guarded = (tell: () => void) => {
      try { tell(); } catch (error) { joined.listeners.delete(mine); fail(error); }
    };
    const mine: ProgressiveListener = { publish: value => guarded(() => { if (!signal.aborted) publish(told(value)); }),
      pending: pending && (() => guarded(pending)) };
    try {
      joined.users++;
      joined.listeners.add(mine);
      // A mount joining late is told what the read has published so far.
      if (joined.latest) mine.publish(joined.latest.value);
      if (joined.waiting) mine.pending?.();
      const value = await untilAborted(Promise.race([joined.done, failed]), signal);
      return told(value);
    } finally {
      joined.listeners.delete(mine);
      joined.users--;
      // React effect replay can rejoin before shared work is cancelled.
      queueMicrotask(() => {
        if (joined.users) return;
        if (reads.get(key) === joined) reads.delete(key);
        joined.controller.abort();
      });
    }
  }
  const held = settled();
  if (held) {
    perfLog?.cached(request, 'hit');
    return untilAborted(held, signal).then(reply => {
      if (signal.aborted) throw new DOMException('Request cancelled', 'AbortError');
      return told(reply.data);
    });
  }
  let final = true;
  // A load going in the other cache is joined there: one request, kept where it was started.
  const other = cache(client, !hours);
  const holder = !replies.holds(key) && other.holds(key) ? other : replies;
  if (perfLog && holder.holds(key)) perfLog.cached(request, 'joined');
  return holder.get(key, async shared => {
    const reply = prepared ? { ...request, data: await preparedSeriesQuery(client, source ?? { id: request.sourceId, revision: request.revision } as DashboardSource,
      request.product as Parameters<typeof preparedSeriesQuery>[2], request.params, shared, restart) }
      : validateProductReply(await client.product<unknown>(request, shared), request);
    final = !prepared || finalPreparedReply(reply.data);
    // A reply this load's validation refuses fails it, and is never held.
    told(reply.data);
    return reply;
  }, signal, () => final).then(reply => told(reply.data));
}

/** Prepared smart groups share the ordinary content/source cache and cancellation.
 * With `publish`, each reply answered from the prepared prefix (carried on by
 * later keyframes) is published as it grows, and the promise resolves with the
 * complete reply; without it, the first reply. */
export function loadCountryGroups(client: DashboardClient, source: import('./client').DashboardSource, branchId: string, signal: AbortSignal, restart = false,
  publish?: (data: import('../generated/dashboard').CountryGroupReply) => void): Promise<import('../generated/dashboard').CountryGroupReply> {
  const request = { sourceId: source.id, revision: source.revision, product: 'country-groups', params: { branchId } };
  return loadProduct(client, request, signal, validateCountryGroupReply, source, restart, publish);
}

/** Load one product without caching it: for per-hour requests that follow playback
 * and would otherwise push the full-branch products out of the shared cache. */
export async function loadTransientProduct<T>(client: DashboardClient, request: ProductRequest, signal: AbortSignal, source?: DashboardSource): Promise<T> {
  if (usesPrepared(client, request, source)) return await preparedSeriesQuery(client, source ?? { id: request.sourceId, revision: request.revision } as DashboardSource, request.product as Parameters<typeof preparedSeriesQuery>[2], request.params, signal) as T;
  return validateProductReply(await client.product<unknown>(request, signal), request).data as T;
}

/** Forget every cached product of a source when it is released; in-flight loads are cancelled. */
export function forgetProducts(client: DashboardClient, sourceId: string): void {
  cache(client).deleteSource(sourceId);
  cache(client, true).deleteSource(sourceId);
  forgetFlags(client, sourceId);
  forgetFacts(client, sourceId);
  forgetTimelines(client, sourceId);
  forgetReplySizes(client, sourceId);
}

/** Forget completed products of a source so they are requested again; loads in flight are untouched. */
export function forgetSettledProducts(client: DashboardClient, sourceId: string): void {
  cache(client).deleteSettled(sourceId);
  cache(client, true).deleteSettled(sourceId);
  forgetFlags(client, sourceId, true);
  forgetFacts(client, sourceId);
  forgetTimelines(client, sourceId, true);
}

export interface ProductState<T> { data?: T; error?: Failure; retry: () => void }
const NO_ERRORS: readonly Failure[] = [];

/** A failed request stays failed until Retry, even while other requests publish. */
export function useProducts<T>(client: DashboardClient, requests: readonly ProductRequest[], active: boolean, validate?: (data: unknown) => T, source?: DashboardSource): ProductState<T[]> {
  const content = useContentRevision(client);
  // Every request uses the current content revision, never its array index.
  const key = JSON.stringify(requests.map(request => productKey(request, content, !!source && !!client.preparedQuery)));
  const [state, setState] = useState<{ client: DashboardClient; key: string; data?: T[]; errors?: readonly Failure[] }>();
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!active || !requests.length) return;
    const controller = new AbortController();
    const values = new Map<number, T>();
    const failures = new Map<number, Failure>();
    const commit = () => {
      if (controller.signal.aborted) return;
      const data = values.size === requests.length ? requests.map((_, index) => values.get(index)!) : undefined;
      if (!data && !failures.size) return;
      const errors = failures.size ? [...new Set(failures.values())] : undefined;
      inFrame(() => controller.signal.aborted || setState(previous => {
        const kept = previous?.client === client && previous.key === key ? previous : undefined;
        const shown = data ?? kept?.data;
        const sameData = kept?.data === shown || !!shown && !!kept?.data && kept.data.every((value, index) => value === shown[index]);
        const before = kept?.errors ?? NO_ERRORS, after = errors ?? NO_ERRORS;
        const sameErrors = before.length === after.length && after.every((error, index) => error === before[index]);
        return kept && sameData && sameErrors ? kept : { client, key, data: shown, errors };
      }));
    };
    requests.forEach((request, index) => {
      const publish = (value: T) => { if (!controller.signal.aborted) { values.set(index, value); commit(); } };
      void loadProduct<T>(client, request, controller.signal, validate, source, attempt > 0, publish).then(publish,
        error => { if (!controller.signal.aborted) { failures.set(index, asFailure(error)); commit(); } });
    });
    return () => controller.abort();
    // The structured key names the requests and their order.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, key, active, attempt]);
  const current = requests.length && state?.client === client && state.key === key ? state : undefined;
  const retry = () => { setState(undefined); setAttempt(value => value + 1); };
  const held = current?.data !== undefined;
  const errors = current?.errors ?? NO_ERRORS;
  const framed = useHeldFailures(held ? errors : NO_ERRORS, retry);
  return { data: current?.data, error: framed && held ? undefined : errors[0], retry };
}

/** Inside a frame, later failures leave published data intact and are reported by the frame. */
export function useProduct<T>(client: DashboardClient, request: ProductRequest | null, active: boolean, validate?: (data: unknown) => T, source?: DashboardSource): ProductState<T> {
  const content = useContentRevision(client);
  const key = request ? productKey(request, content,!!source) : null;
  const [state, setState] = useState<{ client: DashboardClient; key: string; data?: T; error?: Failure }>();
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!active || !request || !key) return;
    const controller = new AbortController();
    const publish = (data: T) => inFrame(() => controller.signal.aborted || setState(previous =>
      previous?.client === client && previous.key === key && previous.data === data && !previous.error ? previous : { client, key, data }));
    void loadProduct<T>(client, request, controller.signal, validate, source, attempt > 0, publish).then(
      publish,
      error => inFrame(() => controller.signal.aborted || setState(previous => ({ client, key,
        data: previous?.client === client && previous.key === key ? previous.data : undefined,
        error: asFailure(error) }))),
    );
    return () => controller.abort();
    // The request object is recreated each render; its key is the identity.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, client, key, attempt]);
  const current = state?.client === client && state.key === key ? state : undefined;
  // A failed load already left the cache; retrying only asks again.
  const retry = () => { setState(undefined); setAttempt(value => value + 1); };
  const error = useHeldFailure(current?.error, current?.data !== undefined, retry);
  return { data: current?.data, error, retry };
}

/** `useProduct` for a read that follows playback: a change to the `moving` params (its
 * hour) does not cancel the running read, which lands and stays shown while the newest
 * moment is read. Cancelled reads hold a connection until they end, so restarting a
 * slow read every hour would starve every other widget. `moment` names the shown reply, `asking` the one in flight. */
export function useFollowedProduct<T>(client: DashboardClient, request: ProductRequest | null, moving: readonly string[], active: boolean,
  validate?: (data: unknown) => T, source?: DashboardSource): ProductState<T> & { moment?: string; asking?: string } {
  const content = useContentRevision(client);
  const params = (request?.params ?? {}) as Record<string, unknown>;
  const lane = request ? JSON.stringify([request.sourceId, request.revision, request.product, !!source, content,
    Object.entries(params).filter(([name]) => !moving.includes(name))]) : null;
  const viewKey = JSON.stringify(moving.map(name => params[name]));
  const read = useViewRead<ProductRequest, T>(lane, request, viewKey, active,
    (at, signal, publish, restart) => loadProduct<T>(client, at, signal, validate, source, restart, publish, undefined, true), client);
  // A failure belongs to the moment it was asked for; a newer moment is still loading.
  const error = useHeldFailure(read.errorKey === viewKey ? read.error : undefined, read.data !== undefined, read.retry);
  return { data: read.data, error, retry: read.retry, moment: read.viewKey, asking: read.asking };
}

/** `useProducts` for requests that follow playback, read as `useFollowedProduct` reads one:
 * the last set stays shown while the newest moment's set is read. `requests` are the
 * ones `data` answers, index for index. */
export function useFollowedProducts<T>(client: DashboardClient, requests: readonly ProductRequest[], moving: readonly string[], active: boolean,
  validate?: (data: unknown) => T, source?: DashboardSource): ProductState<T[]> & { requests?: readonly ProductRequest[] } {
  const content = useContentRevision(client);
  const params = (request: ProductRequest) => (request.params ?? {}) as Record<string, unknown>;
  const lane = requests.length ? JSON.stringify([content, !!source, requests.map(request => [request.sourceId, request.revision, request.product,
    Object.entries(params(request)).filter(([name]) => !moving.includes(name))])]) : null;
  const viewKey = JSON.stringify(requests.map(request => moving.map(name => params(request)[name])));
  const read = useViewRead<readonly ProductRequest[], { requests: readonly ProductRequest[]; values: T[] }>(lane, requests, viewKey,
    active, (at, signal, publish, restart) => {
      // One failure ends the whole set: its other reads stop and never publish.
      const batch = new AbortController();
      const stop = () => batch.abort();
      signal.addEventListener('abort', stop, { once: true });
      const values: T[] = [];
      const published = new Set<number>();
      return Promise.all(at.map((request, index) => loadProduct<T>(client, request, batch.signal, validate, source, restart, value => {
        if (batch.signal.aborted) return;
        values[index] = value; published.add(index);
        if (published.size === at.length) publish({ requests: at, values: [...values] });
      }, undefined, true))).then(results => ({ requests: at, values: results }), error => { batch.abort(); throw error; })
        .finally(() => signal.removeEventListener('abort', stop));
    }, client);
  const error = useHeldFailure(read.errorKey === viewKey ? read.error : undefined, read.data !== undefined, read.retry);
  return { data: read.data?.values, requests: read.data?.requests, error, retry: read.retry };
}
