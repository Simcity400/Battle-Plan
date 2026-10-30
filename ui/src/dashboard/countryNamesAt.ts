import { useEffect, useMemo, useRef, useState } from 'react';
import type { CountryStyleReply } from '../generated/dashboard';
import type { DashboardClient } from './client';
import { FAILED_FACT, FactRequests, factsKey, HourStore, MAX_NAME_HOURS, nameKey, readFactBatch, readFacts } from './countryFacts.ts';
import type { BatchFactSpec, FactRead, FactRequestOrder, FactSpec, FactsFound, NameAt } from './countryFacts';
import { useContentRevision } from './contentRevision.ts';
import type { BoundTrack } from './model';
import { heldFacts, heldTimeline, watchTimeline } from './countryTimeline.ts';
import { inFrame } from './frameUpdates.ts';
import type { CountryTimeline, HeldLookup } from './countryTimeline';

export { MAX_NAME_HOURS, nameKey };
export type { NameAt };

/** How a store's facts are read: per hour and chunk of countries, or every hour in one request. */
type AnyFactSpec<V> = FactSpec<V, FactsFound<V>> | BatchFactSpec<V>;

const sameWanted = (a: readonly NameAt[], b: readonly NameAt[]) => a === b
  || a.length === b.length && a.every((item, index) => item.hour === b[index].hour && item.tag === b[index].tag);

const NO_ANSWERS: ReadonlyMap<string, never> = new Map<string, never>();
const NO_WANTED: readonly NameAt[] = [];

/** Of `wanted`, what `timeline` answers, by `nameKey` (null for a country it gives no value), and
 * the rest, whose hours it does not hold. `before` holds answers from the same timeline, so an
 * answer keeps its identity while it is wanted. */
export function answerHeld<V>(held: HeldLookup<V> | undefined, timeline: CountryTimeline | undefined, wanted: readonly NameAt[],
  before: ReadonlyMap<string, V | null> | undefined): { answered: ReadonlyMap<string, V | null>; rest: readonly NameAt[] } {
  if (!held || !timeline) return { answered: NO_ANSWERS, rest: wanted };
  const answered = new Map<string, V | null>();
  const missing = new Map<number, string[]>();
  for (const item of wanted) {
    const key = nameKey(item.hour, item.tag);
    if (answered.has(key)) continue;
    if (before?.has(key)) { answered.set(key, before.get(key)!); continue; }
    const tags = missing.get(item.hour);
    if (tags) tags.push(item.tag); else missing.set(item.hour, [item.tag]);
  }
  const unheld = new Set<number>();
  for (const [hour, tags] of missing) {
    const found = held(timeline, hour, tags);
    if (!found) { unheld.add(hour); continue; }
    for (const tag of tags) answered.set(nameKey(hour, tag), found.get(tag) ?? null);
  }
  return { answered, rest: !unheld.size ? NO_WANTED : unheld.size === missing.size && !answered.size ? wanted : wanted.filter(item => unheld.has(item.hour)) };
}

/** Facts about countries at the hours `wanted` names, read with `spec` (one request
 * per hour and chunk of countries; every hour wanted unless `maxHours` bounds them,
 * earliest first). Each
 * hour and country is read once while its hour is kept. Every hour wanted now is
 * kept, however many, so wanting more never makes the hook read again what it
 * still wants; of hours no longer wanted it keeps the [`MAX_NAME_HOURS`] used most
 * recently, and none once nothing is wanted (a tooltip closed). A new track starts
 * afresh. A read that failed is tried again when `attempt` changes. Returns only
 * the values `wanted` asks for, keyed by `nameKey`; one the recording does not give
 * at its hour is absent, so the caller shows its fallback. */
export function useCountryFactsAt<V>(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean,
  spec: AnyFactSpec<V>, maxHours = Infinity, attempt = 0): ReadonlyMap<string, V> {
  const { held, answered, rest, version } = useFactStore(client, track, wanted, active, spec, maxHours, attempt);
  // Only what is wanted is handed out, so an update never copies everything read.
  return useMemo(() => {
    const picked = held.pick(rest);
    for (const [key, value] of answered) if (value !== null) picked.set(key, value);
    return picked;
  }, [held, answered, rest, version]); // eslint-disable-line react-hooks/exhaustive-deps
}

/** Like `useCountryFactsAt`, but distinguishes pending (absent), failed
 * (`FAILED_FACT`), and completed without a value (null). A new `attempt` retries
 * failures at once; new content discards every dependent answer. */
export function useCountryFactsRead<V>(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean,
  spec: AnyFactSpec<V>, maxHours = Infinity, attempt = 0): ReadonlyMap<string, V | null | typeof FAILED_FACT> {
  const { requests, answered, rest, version } = useFactStore(client, track, wanted, active, spec, maxHours, attempt);
  return useMemo(() => {
    const picked = requests.pickRead(rest);
    for (const [key, value] of answered) picked.set(key, value);
    return picked;
  }, [requests, answered, rest, version]); // eslint-disable-line react-hooks/exhaustive-deps
}

/** The country timeline held for `track` under `content`, re-rendering, once a frame, as it arrives
 * or grows while `watching`. Reading it never asks for it (`askTimeline` does). */
function useHeldTimeline(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, content: number,
  watching: boolean): CountryTimeline | undefined {
  const [, told] = useState(0);
  const sourceId = track?.source.id, revision = track?.source.revision, branchId = track?.branch.id;
  const timeline = sourceId !== undefined && revision !== undefined && branchId !== undefined
    ? heldTimeline(client, { sourceId, revision, branchId, content }) : undefined;
  const shown = useRef(timeline);
  shown.current = timeline;
  useEffect(() => {
    if (!watching || sourceId === undefined || revision === undefined || branchId === undefined) return;
    const changed = () => inFrame(() => told(value => value + 1));
    const stop = watchTimeline(client, { sourceId, revision, branchId }, changed);
    // A timeline that arrived after the render and before this watch is told now.
    if (heldTimeline(client, { sourceId, revision, branchId, content }) !== shown.current) changed();
    return stop;
  }, [watching, client, sourceId, revision, branchId, content]);
  return timeline;
}

/** The store behind both hooks. Hours the recording's country timeline holds are answered as
 * the hook renders, never read or told a frame later; the store reads only the rest. */
function useFactStore<V>(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean,
  spec: AnyFactSpec<V>, maxHours: number, attempt: number) {
  // Structured, so no tag (a mod's tags may hold any character) can blur two requests.
  const content = useContentRevision(client);
  const trackKey = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, spec.product, spec.identity, content]) : '';
  // The same countries in another order are the same request (`factsKey`). Callers
  // rebuild `wanted` each render, so it is keyed again only when its items change.
  const order = 'batch' in spec ? undefined : spec.requestOrder;
  const keyed = useRef<{ trackKey: string; wanted: readonly NameAt[]; order?: FactRequestOrder; key: string } | null>(null);
  if (!keyed.current || keyed.current.trackKey !== trackKey || keyed.current.order !== order
    || !sameWanted(keyed.current.wanted, wanted)) {
    keyed.current = { trackKey, wanted, order, key: factsKey(trackKey, wanted, order) };
  }
  const key = keyed.current.key;
  const [version, setVersion] = useState(0);
  const timeline = useHeldTimeline(client, track, content, active && !!spec.held);
  // One store and one set of reads per track and client; a change of what is wanted
  // asks only for what is new and cancels nothing (`FactRequests`).
  const store = useRef<{ trackKey: string; client: DashboardClient; store: HourStore<V>; requests: FactRequests<V>;
    held?: { timeline: CountryTimeline | undefined; answered: ReadonlyMap<string, V | null> } } | null>(null);
  if (!store.current || store.current.trackKey !== trackKey || store.current.client !== client) {
    store.current?.requests.cancel();
    const held = new HourStore<V>(MAX_NAME_HOURS);
    const source = track ? { sourceId: track.source.id, revision: track.source.revision, branchId: track.branch.id, content } : null;
    const read: FactRead<V> = (plan, signal, onChunk) => !source ? Promise.resolve()
      : 'batch' in spec ? readFactBatch(client, source, spec, plan, signal, onChunk, track?.source) : readFacts(client, source, spec, plan, signal, onChunk, track?.source);
    store.current = { trackKey, client, store: held, requests: new FactRequests(held, read, () => setVersion(value => value + 1)) };
  }
  const current = store.current;
  const { store: held, requests } = current;
  const split = useMemo(() => {
    const kept = current.held;
    // Another timeline is read anew: a re-read may revise what it held, such as names localised again.
    const before = kept && kept.timeline === timeline ? kept.answered : undefined;
    const split = answerHeld(spec.held, timeline, keyed.current!.wanted, before);
    current.held = { timeline, answered: split.answered };
    return { ...split, key: split.rest === keyed.current!.wanted ? key : factsKey(trackKey, split.rest, order) };
  }, [current, timeline, key, spec.held]); // eslint-disable-line react-hooks/exhaustive-deps
  const { answered, rest } = split;
  const attempted = useRef(attempt);
  // Reads stop when the widget goes away or another track takes over.
  useEffect(() => () => requests.cancel(), [requests]);
  useEffect(() => {
    // Off screen, nothing is read.
    if (!active || !track) { requests.cancel(); return; }
    // Nothing left to read: whatever was read is released.
    if (!rest.length) { requests.cancel(); if (held.size) { held.clear(); setVersion(value => value + 1); } return; }
    // A batched product reads every hour and country queued in one request.
    if ('batch' in spec) requests.want(rest, maxHours, Infinity, 'time', true);
    else requests.want(rest, maxHours, spec.perRequest, spec.requestOrder);
    // A new attempt tries a failing read again without waiting.
    if (attempted.current !== attempt) { attempted.current = attempt; requests.retry(); }
    // The key names every request; the arrays behind it are rebuilt each render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [split.key, active, attempt, requests, order]);
  return { held, requests, answered, rest, version };
}

/** Names from `country-styles`, which names at most 64 countries per request. */
const NAMES: FactSpec<string> = {
  product: 'country-styles', perRequest: 64, shared: true,
  params: (branchId, hour, tags) => ({ branchId, hour, tags }),
  extract: data => new Map((data as CountryStyleReply).styles.flatMap(style => typeof style.name === 'string' ? [[style.tag, style.name] as const] : [])),
  held: heldFacts(style => style.name),
};

/** Country names as recorded at the given hours (`country-styles`), for labels
 * dated to an event; see `useCountryFactsAt`. */
export function useCountryNamesAt(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean,
  maxHours = Infinity, attempt = 0): ReadonlyMap<string, string> {
  return useCountryFactsAt(client, track, wanted, active, NAMES, maxHours, attempt);
}

/** Name reads retain pending, failed and completed-missing states for dated labels. */
export function useCountryNamesRead(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean,
  maxHours = Infinity, attempt = 0): ReadonlyMap<string, string | null | typeof FAILED_FACT> {
  return useCountryFactsRead(client, track, wanted, active, NAMES, maxHours, attempt);
}
