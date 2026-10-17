import { useEffect, useState } from 'react';
import type { DashboardClient, DashboardSource, ProductReply, ProductRequest } from './client';
import { validateProductReply } from './client.ts';
import { ResourceCache } from './resourceCache.ts';
import { forgetFlags } from './flagCache.ts';
import { contentRevision, useContentRevision } from './contentRevision.ts';
import { isPreparedProduct, preparedSeriesQuery } from './preparedSeries.ts';
import { validateCountryGroupReply } from './groupsReply.ts';
import { CONTENT_PRODUCTS } from '../generated/dashboard.ts';
import { newId } from '../shell/ids.ts';
import { finalPreparedReply } from './preparedPrefix.ts';

/** Each reply's length as the transport received it, so the cache sizes a reply
 * without serialising it again; a reply without one is measured by serialising. */
export const replyLengths = new WeakMap<object, number>();

/** One shared, bounded cache of final product replies per client. One-shot
 * requests share native work; progressive replies and polling belong to each mount. */
const caches = new WeakMap<DashboardClient, ResourceCache<ProductReply<unknown>>>();
function cache(client: DashboardClient): ResourceCache<ProductReply<unknown>> {
  let entry = caches.get(client);
  if (!entry) {
    entry = new ResourceCache(128, 64 * 1024 * 1024, reply => (replyLengths.get(reply) ?? JSON.stringify(reply).length) * 2);
    caches.set(client, entry);
  }
  return entry;
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
  const prepared = isPreparedProduct(request.product) || (!!client.preparedQuery && request.product === 'politics-ledger') || (!!source && !!client.preparedQuery && (request.product === 'army-size' || request.product === 'supply-health' || request.product === 'series-summary' || request.product === 'military-air-ledger' || request.product === 'air-aces' || request.product === 'air-ace-portraits' || request.product === 'raids' || request.product === 'air-superiority' || request.product === 'casualty-pairs' || request.product === 'equipment-losses' || request.product === 'subjects' || request.product === 'wars' || request.product === 'country-groups' || request.product === 'party-history' || request.product === 'scoreboard' || request.product === 'series-state' || request.product === 'unit-leaders' || request.product === 'construction-log' || request.product === 'construction-log-manifest' || request.product === 'construction-spending' || request.product === 'resource-balance' || request.product === 'trade-flows' || request.product === 'production-spending' || request.product === 'casualty-breakdown' || request.product === 'equipment-stockpile' || request.product === 'series-next-change' || request.product === 'events-page' || request.product === 'events-clusters' || request.product === 'occupied-states' || request.product === 'occupied-states-manifest' || request.product === 'land-owners' || request.product === 'espionage-log' || request.product === 'operative-placements' || request.product === 'operatives-roster' || request.product === 'agency-comparison' || request.product === 'decryption'));
  if (prepared && source && (source.id !== request.sourceId || source.revision !== request.revision)) throw new Error('The prepared product source does not match its request.');
  return prepared;
}

/** One-shot loads return their first reply. Mounted prepared loads publish each
 * accepted extent; only final replies enter the shared bounded cache. */
export async function loadProduct<T>(client: DashboardClient, request: ProductRequest, signal: AbortSignal, validate?: (data: unknown) => T, source?: DashboardSource, restart = false, publish?: (data: T) => void): Promise<T> {
  const prepared = usesPrepared(client, request, source);
  const replies = cache(client), key = productKey(request, contentRevision(client).snapshot(), prepared);
  const validated = (reply: ProductReply<unknown>): ProductReply<unknown> => {
    if (!validate) return reply;
    const value = { ...reply, data: validate(reply.data) };
    const length = replyLengths.get(reply);
    if (length !== undefined) replyLengths.set(value, length);
    return value;
  };
  if (prepared && publish) {
    if (signal.aborted) throw new DOMException('Request cancelled', 'AbortError');
    const cached = replies.getSettled(key);
    if (cached) {
      const reply = await cached;
      if (signal.aborted) throw new DOMException('Request cancelled', 'AbortError');
      publish(reply.data as T);
      return reply.data as T;
    }
    // Each mount owns its transient replies and cancellation. The existing
    // query loop alone polls accepted coverage; no partial reply is cached.
    let latest: ProductReply<unknown> | undefined, latestRaw: unknown;
    // A unique in-flight entry keeps source release cancellation in the same
    // cache machinery. It retains no result; the final reply uses the real key.
    const reply = await replies.get(JSON.stringify([request.sourceId, key, newId()]), async shared => {
      const data = await preparedSeriesQuery(client, source ?? { id: request.sourceId, revision: request.revision } as DashboardSource,
        request.product as Parameters<typeof preparedSeriesQuery>[2], request.params, shared, restart, undefined, undefined, value => {
          latestRaw = value;
          latest = validated({ ...request, data: value });
          if (!signal.aborted) publish(latest.data as T);
        });
      if (signal.aborted || shared.aborted) throw new DOMException('Request cancelled', 'AbortError');
      const reply = latest && latestRaw === data ? latest : validated({ ...request, data });
      if (finalPreparedReply(data)) replies.put(key, reply);
      return reply;
    }, signal, () => false);
    return reply.data as T;
  }
  let final = true;
  return replies.get(key, async shared => {
    const reply = prepared ? { ...request, data: await preparedSeriesQuery(client, source ?? { id: request.sourceId, revision: request.revision } as DashboardSource,
      request.product as Parameters<typeof preparedSeriesQuery>[2], request.params, shared, restart) }
      : validateProductReply(await client.product<unknown>(request, shared), request);
    final = !prepared || finalPreparedReply(reply.data);
    return validated(reply);
  }, signal, () => final).then(reply => reply.data as T);
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
  forgetFlags(client, sourceId);
}

/** Forget completed products of a source so they are requested again; loads in flight are untouched. */
export function forgetSettledProducts(client: DashboardClient, sourceId: string): void {
  cache(client).deleteSettled(sourceId);
  forgetFlags(client, sourceId);
}

export interface ProductState<T> { data?: T; error?: string; retry: () => void }

/** Several independent country replies, on the same shared cache as useProduct.
 * Request order is reply order; one failure never presents a partial comparison
 * as complete. Playback still performs only local lookups. */
export function useProducts<T>(client: DashboardClient, requests: readonly ProductRequest[], active: boolean, validate?: (data: unknown) => T, source?: DashboardSource): ProductState<T[]> {
  const content = useContentRevision(client);
  // Every request uses the current content revision, never its array index.
  const key = JSON.stringify(requests.map(request => productKey(request, content, !!source && !!client.preparedQuery)));
  const [state, setState] = useState<{ client: DashboardClient; key: string; data?: T[]; error?: string }>();
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!active || !requests.length) return;
    const controller = new AbortController();
    const values = new Map<number, T>();
    const publish = (index: number, value: T) => {
      if (controller.signal.aborted) return;
      values.set(index, value);
      if (values.size !== requests.length) return;
      const data = requests.map((_, index) => values.get(index)!);
      setState(previous => previous?.client === client && previous.key === key && !previous.error
        && previous.data?.every((value, index) => value === data[index]) ? previous : { client, key, data });
    };
    void Promise.all(requests.map((request, index) => loadProduct<T>(client, request, controller.signal, validate, source, attempt > 0,
      value => publish(index, value)).then(value => { publish(index, value); }))).catch(
      error => { if (!controller.signal.aborted) setState(previous => ({ client, key,
        data: previous?.client === client && previous.key === key ? previous.data : undefined,
        error: error instanceof Error ? error.message : String(error) })); },
    );
    return () => controller.abort();
    // The structured key names the requests and their order.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, key, active, attempt]);
  const current = requests.length && state?.client === client && state.key === key ? state : undefined;
  return { data: current?.data, error: current?.error, retry: () => { setState(undefined); setAttempt(value => value + 1); } };
}

/** Widgets read a product with this hook; a null request clears the state. */
export function useProduct<T>(client: DashboardClient, request: ProductRequest | null, active: boolean, validate?: (data: unknown) => T, source?: DashboardSource): ProductState<T> {
  const content = useContentRevision(client);
  const key = request ? productKey(request, content,!!source) : null;
  const [state, setState] = useState<{ client: DashboardClient; key: string; data?: T; error?: string }>();
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!active || !request || !key) return;
    const controller = new AbortController();
    const publish = (data: T) => { if (!controller.signal.aborted) setState(previous =>
      previous?.client === client && previous.key === key && previous.data === data && !previous.error ? previous : { client, key, data }); };
    void loadProduct<T>(client, request, controller.signal, validate, source, attempt > 0, publish).then(
      publish,
      error => { if (!controller.signal.aborted) setState(previous => ({ client, key,
        data: previous?.client === client && previous.key === key ? previous.data : undefined,
        error: error instanceof Error ? error.message : String(error) })); },
    );
    return () => controller.abort();
    // The request object is recreated each render; its key is the identity.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, client, key, attempt]);
  const current = state?.client === client && state.key === key ? state : undefined;
  return {
    data: current?.data,
    error: current?.error,
    // A failed load already left the cache; retrying only asks again.
    retry: () => { setState(undefined); setAttempt(value => value + 1); },
  };
}
