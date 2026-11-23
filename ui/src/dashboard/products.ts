import { type Failure, asFailure } from './queryError.ts';
import { useEffect, useRef, useState } from 'react';
import type { DashboardClient, DashboardSource, ProductReply, ProductRequest } from './client';
import { validateProductReply } from './client.ts';
import { ResourceCache } from './resourceCache.ts';
import { forgetFlags } from './countryFlags.ts';
import { forgetFacts } from './sharedFacts.ts';
import { forgetTimelines } from './countryTimeline.ts';
import { forgetReplySizes } from './replySizes.ts';
import { tileCache } from './tiles/view.ts';
import { contentRevision, useContentRevision } from './contentRevision.ts';
import { validateCountryGroupReply } from './groupsReply.ts';
import { pageJoin } from './pageJoins.ts';
import { CONTENT_PRODUCTS } from '../generated/dashboard.ts';
import { useHeldFailure, useHeldFailures } from './heldFailures.ts';
import { useViewRead, type Holds } from './viewRead.ts';
import { inFrame } from './frameUpdates.ts';
import { perfLog } from '../shell/perf.ts';
import { countRead, countWork, measuredWidget } from './widgetPerf.ts';
import { keepDigest } from './replyDigest.ts';
import { askedByPage, baseClient, noteRead } from './prepare.ts';
import { productRequest } from './request.ts';

export const replyLengths = new WeakMap<object, number>();

function receivedLength(reply: ProductReply<unknown>): number | undefined {
  const data: unknown = reply.data;
  return replyLengths.get(reply) ?? (data !== null && typeof data === 'object' ? replyLengths.get(data) : undefined);
}

const caches = new WeakMap<DashboardClient, { shared: ResourceCache<ProductReply<unknown>>; hours: ResourceCache<ProductReply<unknown>> }>();
const replySize = (reply: ProductReply<unknown>) => (receivedLength(reply) ?? JSON.stringify(reply).length) * 2;
function cache(client: DashboardClient, hours = false): ResourceCache<ProductReply<unknown>> {
  client = baseClient(client);
  let entry = caches.get(client);
  if (!entry) {
    entry = { shared: new ResourceCache(1024, 256 * 1024 * 1024, replySize), hours: new ResourceCache(512, 64 * 1024 * 1024, replySize) };
    caches.set(client, entry);
  }
  return hours ? entry.hours : entry.shared;
}

const contentProducts: ReadonlySet<string> = new Set(CONTENT_PRODUCTS);

export function productKey(request: ProductRequest, content = 0): string {
  const key: unknown[] = [request.sourceId, request.revision, request.product, request.params];
  if (contentProducts.has(request.product)) key.push(content);
  return JSON.stringify(key);
}

const validations = new WeakMap<object, WeakMap<(data: unknown) => unknown, { data: unknown } | { refused: unknown }>>();
function validatedData<T>(data: unknown, validate?: (data: unknown) => T): T {
  if (!validate) return data as T;
  if (data === null || typeof data !== 'object') return validate(data);
  let made = validations.get(data);
  if (!made) validations.set(data, made = new WeakMap());
  let result = made.get(validate);
  if (!result) {
    try { result = { data: keepDigest(data, validate(data)) }; } catch (refused) { result = { refused }; }
    made.set(validate, result);
  }
  if ('refused' in result) throw result.refused;
  return result.data as T;
}

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
  if (request.product !== 'series-summary') return null;
  const kind = (request.params as { family?: { kind?: unknown } } | null)?.family?.kind;
  return typeof kind === 'string' ? kind : null;
}

export async function loadProduct<T>(client: DashboardClient, request: ProductRequest, signal: AbortSignal, validate?: (data: unknown) => T, source?: DashboardSource, restart = false, publish?: (data: T) => void, pending?: () => void, hours = false): Promise<T> {
  const kind = pagedKind(request);
  if (kind === null) return loadPage(client, request, signal, validate, source, restart, publish, pending, hours);
  const join = pageJoin(kind);
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

async function loadPage<T>(client: DashboardClient, request: ProductRequest, signal: AbortSignal, validate?: (data: unknown) => T, _source?: DashboardSource, _restart = false, publish?: (data: T) => void, _pending?: () => void, hours = false): Promise<T> {
  const replies = cache(client, hours), key = productKey(request, contentRevision(client).snapshot());
  noteRead(client, request);
  const settled = () => replies.getSettled(key) ?? cache(client, !hours).getSettled(key);
  const told = (data: unknown) => validatedData(data, validate);
  const held = settled();
  if (held) {
    perfLog?.cached(request, 'hit');
    return untilAborted(held, signal).then(reply => {
      if (signal.aborted) throw new DOMException('Request cancelled', 'AbortError');
      const data = told(reply.data);
      publish?.(data);
      return data;
    });
  }
  const other = cache(client, !hours);
  const holder = !replies.holds(key) && other.holds(key) ? other : replies;
  if (holder.holds(key)) {
    perfLog?.cached(request, 'joined');
    askedByPage(client, request);
  }
  const accepted = (reply: ProductReply<unknown>) => { try { told(reply.data); return true; } catch { return false; } };
  return holder.get(key, async shared => validateProductReply(await client.product<unknown>(request, shared), request), signal, accepted).then(reply => {
    const data = told(reply.data);
    publish?.(data);
    return data;
  });
}

export function peekProduct<T>(client: DashboardClient, request: ProductRequest, key: string, validate?: (data: unknown) => T): T | undefined {
  if (pagedKind(request) !== null) return undefined;
  const reply = cache(client).peek(key) ?? cache(client, true).peek(key);
  if (!reply) return undefined;
  let byValidator = peeked.get(reply);
  if (!byValidator) peeked.set(reply, byValidator = new Map());
  const as = validate ?? NO_VALIDATION;
  if (byValidator.has(as)) return byValidator.get(as) as T | undefined;
  let data: T | undefined;
  try { data = validatedData(reply.data, validate); } catch { data = undefined; }
  byValidator.set(as, data);
  return data;
}
const NO_VALIDATION = (data: unknown) => data;
const peeked = new WeakMap<ProductReply<unknown>, Map<(data: unknown) => unknown, unknown>>();

export function pinProducts(client: DashboardClient, owner: object, requests: readonly ProductRequest[]): void {
  const content = contentRevision(client).snapshot();
  const keys = requests.map(request => productKey(request, content));
  cache(client).pin(owner, keys);
  cache(client, true).pin(owner, keys);
}

export function unpinProducts(client: DashboardClient, owner: object): void {
  cache(client).unpin(owner);
  cache(client, true).unpin(owner);
}

export function loadCountryGroups(client: DashboardClient, source: import('./client').DashboardSource, branchId: string, signal: AbortSignal, restart = false,
  publish?: (data: import('../generated/dashboard').CountryGroupReply) => void): Promise<import('../generated/dashboard').CountryGroupReply> {
  const request = productRequest({ sourceId: source.id, revision: source.revision, product: 'country-groups', params: { branchId } });
  return loadProduct(client, request, signal, validateCountryGroupReply, source, restart, publish);
}

export async function loadTransientProduct<T>(client: DashboardClient, request: ProductRequest, signal: AbortSignal, _source?: DashboardSource): Promise<T> {
  return validateProductReply(await client.product<unknown>(request, signal), request).data as T;
}

export function forgetProducts(client: DashboardClient, sourceId: string): void {
  cache(client).deleteSource(sourceId);
  cache(client, true).deleteSource(sourceId);
  forgetFlags(client, sourceId);
  forgetFacts(client, sourceId);
  forgetTimelines(client, sourceId);
  forgetReplySizes(client, sourceId);
  tileCache(client).forget(`${sourceId}|`);
}

export function forgetSettledProducts(client: DashboardClient, sourceId: string): void {
  cache(client).deleteSettled(sourceId);
  cache(client, true).deleteSettled(sourceId);
  forgetFlags(client, sourceId, true);
  forgetFacts(client, sourceId);
  forgetTimelines(client, sourceId, true);
}

export interface ProductState<T> { data?: T; error?: Failure; retry: () => void }

function useRunWhileStarted(active: boolean, key: string | null) {
  const running = useRef<{ key: string } | null>(null);
  return {
    wanted: active || (key !== null && running.current?.key === key),
    started: (at: string) => { const run = { key: at }; running.current = run; return run; },
    settled: (run: { key: string }) => inFrame(() => { if (running.current === run) running.current = null; }),
    cancelled: (run: { key: string }) => { if (running.current === run) running.current = null; },
  };
}
const NO_ERRORS: readonly Failure[] = [];

export function useProducts<T>(client: DashboardClient, requests: readonly ProductRequest[], active: boolean, validate?: (data: unknown) => T, source?: DashboardSource): ProductState<T[]> {
  const content = useContentRevision(client);
  const key = JSON.stringify(requests.map(request => productKey(request, content)));
  const widget = measuredWidget();
  const [state, setState] = useState<{ client: DashboardClient; key: string; data?: T[]; errors?: readonly Failure[] }>();
  const [attempt, setAttempt] = useState(0);
  const run = useRunWhileStarted(active, key);
  useEffect(() => {
    if (!run.wanted || !requests.length) return;
    const controller = new AbortController();
    const values = new Map<number, T>();
    const failures = new Map<number, Failure>();
    let left = requests.length;
    const mine = run.started(key);
    const settled = () => { if (--left === 0) run.settled(mine); };
    const commit = () => {
      if (controller.signal.aborted) return;
      const data = values.size === requests.length ? requests.map((_, index) => values.get(index)!) : undefined;
      if (!data && !failures.size) return;
      const errors = failures.size ? [...new Set(failures.values())] : undefined;
      inFrame(() => {
        if (controller.signal.aborted) return;
        countWork(widget, 'reply');
        setState(previous => {
          const kept = previous?.client === client && previous.key === key ? previous : undefined;
          const shown = data ?? kept?.data;
          const sameData = kept?.data === shown || !!shown && !!kept?.data && kept.data.every((value, index) => value === shown[index]);
          const before = kept?.errors ?? NO_ERRORS, after = errors ?? NO_ERRORS;
          const sameErrors = before.length === after.length && after.every((error, index) => error === before[index]);
          return kept && sameData && sameErrors ? kept : { client, key, data: shown, errors };
        });
      });
    };
    requests.forEach((request, index) => {
      const publish = (value: T) => { if (!controller.signal.aborted) { values.set(index, value); commit(); } };
      countRead(widget, request);
      void loadProduct<T>(client, request, controller.signal, validate, source, attempt > 0, publish).then(publish,
        error => { if (!controller.signal.aborted) { failures.set(index, asFailure(error)); commit(); } }).finally(settled);
    });
    return () => { controller.abort(); run.cancelled(mine); };
  }, [client, key, run.wanted, attempt]);
  const kept = requests.length && state?.client === client && state.key === key ? state : undefined;
  const peeked = useRef<{ key: string; data: T[] | undefined } | null>(null);
  if (!kept?.data && requests.length && peeked.current?.key !== key) {
    const values = requests.map(request => peekProduct(client, request, productKey(request, content), validate));
    peeked.current = { key, data: values.every(value => value !== undefined) ? values as T[] : undefined };
  }
  const current = kept?.data || !peeked.current?.data || peeked.current.key !== key ? kept : { client, key, data: peeked.current.data, errors: undefined };
  const retry = () => { setState(undefined); setAttempt(value => value + 1); };
  const held = current?.data !== undefined;
  const errors = current?.errors ?? NO_ERRORS;
  const framed = useHeldFailures(held ? errors : NO_ERRORS, retry);
  return { data: current?.data, error: framed && held ? undefined : errors[0], retry };
}

export function useProduct<T>(client: DashboardClient, request: ProductRequest | null, active: boolean, validate?: (data: unknown) => T, source?: DashboardSource): ProductState<T> {
  const content = useContentRevision(client);
  const key = request ? productKey(request, content) : null;
  const widget = measuredWidget();
  const [state, setState] = useState<{ client: DashboardClient; key: string; data?: T; error?: Failure }>();
  const [attempt, setAttempt] = useState(0);
  const run = useRunWhileStarted(active, key);
  useEffect(() => {
    if (!run.wanted || !request || !key) return;
    const controller = new AbortController();
    const mine = run.started(key);
    const publish = (data: T) => inFrame(() => {
      if (controller.signal.aborted) return;
      countWork(widget, 'reply');
      setState(previous => previous?.client === client && previous.key === key && previous.data === data && !previous.error ? previous : { client, key, data });
    });
    countRead(widget, request);
    void loadProduct<T>(client, request, controller.signal, validate, source, attempt > 0, publish).then(
      publish,
      error => inFrame(() => controller.signal.aborted || setState(previous => ({ client, key,
        data: previous?.client === client && previous.key === key ? previous.data : undefined,
        error: asFailure(error) }))),
    ).finally(() => run.settled(mine));
    return () => { controller.abort(); run.cancelled(mine); };
  }, [run.wanted, client, key, attempt]);
  const kept = state?.client === client && state.key === key ? state : undefined;
  const peeked = useRef<{ key: string; data: T | undefined } | null>(null);
  if (kept?.data === undefined && request && key && peeked.current?.key !== key) peeked.current = { key, data: peekProduct(client, request, key, validate) };
  const heldData = kept?.data === undefined && key && peeked.current?.key === key ? peeked.current.data : undefined;
  const current = heldData === undefined ? kept : { ...kept, client, key: key!, data: heldData };
  const retry = () => { setState(undefined); setAttempt(value => value + 1); };
  const error = useHeldFailure(current?.error, current?.data !== undefined, retry);
  return { data: current?.data, error, retry };
}

export function useFollowedProduct<T>(client: DashboardClient, request: ProductRequest | null, moving: readonly string[], active: boolean,
  validate?: (data: unknown) => T, source?: DashboardSource, holds?: Holds<ProductRequest, T>): ProductState<T> & { moment?: string; asking?: string } {
  const content = useContentRevision(client);
  const params = (request?.params ?? {}) as Record<string, unknown>;
  const lane = request ? JSON.stringify([request.sourceId, request.revision, request.product, !!source, content,
    Object.entries(params).filter(([name]) => !moving.includes(name))]) : null;
  const viewKey = JSON.stringify(moving.map(name => params[name]));
  const read = useViewRead<ProductRequest, T>(lane, request, viewKey, active,
    (at, signal, publish, restart) => loadProduct<T>(client, at, signal, validate, source, restart, publish, undefined, true), client, holds,
    at => peekProduct(client, at, productKey(at, content), validate));
  const error = useHeldFailure(read.errorKey === viewKey ? read.error : undefined, read.data !== undefined, read.retry);
  return { data: read.data, error, retry: read.retry, moment: read.viewKey, asking: read.asking };
}

export function useFollowedProducts<T>(client: DashboardClient, requests: readonly ProductRequest[], moving: readonly string[], active: boolean,
  validate?: (data: unknown) => T, source?: DashboardSource): ProductState<T[]> & { requests?: readonly ProductRequest[] } {
  const content = useContentRevision(client);
  const params = (request: ProductRequest) => (request.params ?? {}) as Record<string, unknown>;
  const lane = requests.length ? JSON.stringify([content, !!source, requests.map(request => [request.sourceId, request.revision, request.product,
    Object.entries(params(request)).filter(([name]) => !moving.includes(name))])]) : null;
  const viewKey = JSON.stringify(requests.map(request => moving.map(name => params(request)[name])));
  const read = useViewRead<readonly ProductRequest[], { requests: readonly ProductRequest[]; values: T[] }>(lane, requests, viewKey,
    active, (at, signal, publish, restart) => {
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
