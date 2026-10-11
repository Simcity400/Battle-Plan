import { useEffect, useState } from 'react';
import type { DashboardClient, ProductReply, ProductRequest } from './client';
import { validateProductReply } from './client.ts';
import { ResourceCache } from './resourceCache.ts';
import { forgetFlags } from './flagCache.ts';
import { contentRevision, useContentRevision } from './contentRevision.ts';
import { CONTENT_PRODUCTS } from '../generated/dashboard.ts';

/** Each reply's length as the transport received it, so the cache sizes a reply
 * without serialising it again; a reply without one is measured by serialising. */
export const replyLengths = new WeakMap<object, number>();

/** One shared, bounded cache of product replies per client. Identical requests
 * share one native round trip; playback reads the cached full-branch data. */
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

export function productKey(request: ProductRequest, content = 0): string {
  const key: unknown[] = [request.sourceId, request.revision, request.product, request.params];
  if (contentProducts.has(request.product)) key.push(content);
  return JSON.stringify(key);
}

/** Load one product, validating the envelope and, when given, the payload. */
export function loadProduct<T>(client: DashboardClient, request: ProductRequest, signal: AbortSignal, validate?: (data: unknown) => T): Promise<T> {
  return cache(client).get(productKey(request, contentRevision(client).snapshot()), async shared => {
    const reply = validateProductReply(await client.product<unknown>(request, shared), request);
    if (!validate) return reply;
    const validated = { ...reply, data: validate(reply.data) };
    const length = replyLengths.get(reply);
    if (length !== undefined) replyLengths.set(validated, length);
    return validated;
  }, signal).then(reply => reply.data as T);
}

/** Load one product without caching it: for per-hour requests that follow playback
 * and would otherwise push the full-branch products out of the shared cache. */
export async function loadTransientProduct<T>(client: DashboardClient, request: ProductRequest, signal: AbortSignal): Promise<T> {
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
export function useProducts<T>(client: DashboardClient, requests: readonly ProductRequest[], active: boolean, validate?: (data: unknown) => T): ProductState<T[]> {
  const content = useContentRevision(client);
  // Every request uses the current content revision, never its array index.
  const key = JSON.stringify(requests.map(request => productKey(request, content)));
  const [state, setState] = useState<{ client: DashboardClient; key: string; data?: T[]; error?: string }>();
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!active || !requests.length) return;
    const controller = new AbortController();
    void Promise.all(requests.map(request => loadProduct<T>(client, request, controller.signal, validate))).then(
      data => { if (!controller.signal.aborted) setState({ client, key, data }); },
      error => { if (!controller.signal.aborted) setState({ client, key, error: error instanceof Error ? error.message : String(error) }); },
    );
    return () => controller.abort();
    // The structured key names the requests and their order.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, key, active, attempt]);
  const current = requests.length && state?.client === client && state.key === key ? state : undefined;
  return { data: current?.data, error: current?.error, retry: () => { setState(undefined); setAttempt(value => value + 1); } };
}

/** Widgets read a product with this hook; a null request clears the state. */
export function useProduct<T>(client: DashboardClient, request: ProductRequest | null, active: boolean, validate?: (data: unknown) => T): ProductState<T> {
  const content = useContentRevision(client);
  const key = request ? productKey(request, content) : null;
  const [state, setState] = useState<{ client: DashboardClient; key: string; data?: T; error?: string }>();
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!active || !request || !key) return;
    const controller = new AbortController();
    void loadProduct<T>(client, request, controller.signal, validate).then(
      data => { if (!controller.signal.aborted) setState({ client, key, data }); },
      error => { if (!controller.signal.aborted) setState({ client, key, error: error instanceof Error ? error.message : String(error) }); },
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
