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
  client = baseClient(client);
  let entry = caches.get(client);
  if (!entry) {
    // Sized to hold every page's replies for one campaign and country (a 13-year campaign's tour of
    // the pages is about 240 replies and 80 MB of JSON), so a page shown before shows again at once.
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
    try { result = { data: keepDigest(data, validate(data)) }; } catch (refused) { result = { refused }; }
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
  if (request.product !== 'series-summary') return null;
  const kind = (request.params as { family?: { kind?: unknown } } | null)?.family?.kind;
  return typeof kind === 'string' ? kind : null;
}

/** A product's reply, from a bounded cache, the hours' own for a read that follows the playhead
 * (`hours`); mounts asking the same thing share one request. A paged reply is read page after
 * page, each published joined to the pages before it. */
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

async function loadPage<T>(client: DashboardClient, request: ProductRequest, signal: AbortSignal, validate?: (data: unknown) => T, _source?: DashboardSource, _restart = false, publish?: (data: T) => void, _pending?: () => void, hours = false): Promise<T> {
  const replies = cache(client, hours), key = productKey(request, contentRevision(client).snapshot());
  noteRead(client, request);
  // A final reply either cache holds answers at once, whichever kind of read settled it.
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
  // A load going in the other cache is joined there: one request, kept where it was started.
  const other = cache(client, !hours);
  const holder = !replies.holds(key) && other.holds(key) ? other : replies;
  if (holder.holds(key)) {
    perfLog?.cached(request, 'joined');
    askedByPage(client, request);
  }
  // A reply the starting mount's validation refuses is never held; every mount it reaches judges it its own way.
  const accepted = (reply: ProductReply<unknown>) => { try { told(reply.data); return true; } catch { return false; } };
  return holder.get(key, async shared => validateProductReply(await client.product<unknown>(request, shared), request), signal, accepted).then(reply => {
    const data = told(reply.data);
    publish?.(data);
    return data;
  });
}

/** The reply settled under `key` (`productKey`) in either cache, as `validate` makes it, at once; undefined while none
 * is, or the reply is one `validate` refuses, or a paged reply joined from several. */
export function peekProduct<T>(client: DashboardClient, request: ProductRequest, key: string, validate?: (data: unknown) => T): T | undefined {
  if (pagedKind(request) !== null) return undefined;
  const reply = cache(client).peek(key) ?? cache(client, true).peek(key);
  if (!reply) return undefined;
  // One reply peeked again (every render until its read settles) is the same data: a new object each render would
  // re-run every memo and effect keyed on it, a render and a read per frame while playing.
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

/** Holds, past the caches' bounds, the replies of `requests` for `owner` in place of what it held: the prepare lane's
 * replies for each page, so what it holds is what the campaign's pages show at one hour. */
export function pinProducts(client: DashboardClient, owner: object, requests: readonly ProductRequest[]): void {
  const content = contentRevision(client).snapshot();
  const keys = requests.map(request => productKey(request, content));
  cache(client).pin(owner, keys);
  cache(client, true).pin(owner, keys);
}

/** Lets go of what `owner` pinned. */
export function unpinProducts(client: DashboardClient, owner: object): void {
  cache(client).unpin(owner);
  cache(client, true).unpin(owner);
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
export async function loadTransientProduct<T>(client: DashboardClient, request: ProductRequest, signal: AbortSignal, _source?: DashboardSource): Promise<T> {
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
  tileCache(client).forget(`${sourceId}|`);
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

/** Whether a read of `key` should run: while its widget is in view, and, once started, until it settles even
 * when the widget scrolls out of view, so a slow read is never thrown away and asked again from the start.
 * Leaving the page (unmounting) still cancels it. */
function useRunWhileStarted(active: boolean, key: string | null) {
  // Each run is its own object, so an earlier run settling never clears a later one of the same key.
  const running = useRef<{ key: string } | null>(null);
  return {
    wanted: active || (key !== null && running.current?.key === key),
    started: (at: string) => { const run = { key: at }; running.current = run; return run; },
    // Settled in the frame its last reply renders in (frames run in order): a render before then
    // would cancel the run and drop that reply.
    settled: (run: { key: string }) => inFrame(() => { if (running.current === run) running.current = null; }),
    // A cancelled run ends at once: a later ask of its key is a new run, wanted only while in view.
    cancelled: (run: { key: string }) => { if (running.current === run) running.current = null; },
  };
}
const NO_ERRORS: readonly Failure[] = [];

/** A failed request stays failed until Retry, even while other requests publish. */
export function useProducts<T>(client: DashboardClient, requests: readonly ProductRequest[], active: boolean, validate?: (data: unknown) => T, source?: DashboardSource): ProductState<T[]> {
  const content = useContentRevision(client);
  // Every request uses the current content revision, never its array index.
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
    // The structured key names the requests and their order.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, key, run.wanted, attempt]);
  const kept = requests.length && state?.client === client && state.key === key ? state : undefined;
  // Every reply already held shows from the first render, never a loading state; peeked once a key, as `useViewRead` does.
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

/** Inside a frame, later failures leave published data intact and are reported by the frame. */
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
    // The request object is recreated each render; its key is the identity.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [run.wanted, client, key, attempt]);
  const kept = state?.client === client && state.key === key ? state : undefined;
  // A reply already held shows from the first render, never a loading state; peeked once a key, as `useViewRead` does.
  const peeked = useRef<{ key: string; data: T | undefined } | null>(null);
  if (kept?.data === undefined && request && key && peeked.current?.key !== key) peeked.current = { key, data: peekProduct(client, request, key, validate) };
  const heldData = kept?.data === undefined && key && peeked.current?.key === key ? peeked.current.data : undefined;
  const current = heldData === undefined ? kept : { ...kept, client, key: key!, data: heldData };
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
  validate?: (data: unknown) => T, source?: DashboardSource, holds?: Holds<ProductRequest, T>): ProductState<T> & { moment?: string; asking?: string } {
  const content = useContentRevision(client);
  const params = (request?.params ?? {}) as Record<string, unknown>;
  const lane = request ? JSON.stringify([request.sourceId, request.revision, request.product, !!source, content,
    Object.entries(params).filter(([name]) => !moving.includes(name))]) : null;
  const viewKey = JSON.stringify(moving.map(name => params[name]));
  const read = useViewRead<ProductRequest, T>(lane, request, viewKey, active,
    (at, signal, publish, restart) => loadProduct<T>(client, at, signal, validate, source, restart, publish, undefined, true), client, holds,
    at => peekProduct(client, at, productKey(at, content), validate));
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
