import { useEffect, useMemo, useRef, useState } from 'react';
import type { CountryStyleReply } from '../generated/dashboard';
import type { DashboardClient } from './client';
import { FAILED_FACT, FactRequests, factsKey, HourStore, MAX_NAME_HOURS, nameKey, readFacts } from './countryFacts.ts';
import type { FactRead, FactSpec, NameAt } from './countryFacts';
import { useContentRevision } from './contentRevision.ts';
import type { BoundTrack } from './model';

export { MAX_NAME_HOURS, nameKey };
export type { NameAt };

/** Facts about countries at the hours `wanted` names, read with `spec` (one request
 * per hour and chunk of countries, at most `maxHours` hours, earliest first). Each
 * hour and country is read once while its hour is kept. Every hour wanted now is
 * kept, however many, so wanting more never makes the hook read again what it
 * still wants; of hours no longer wanted it keeps the [`MAX_NAME_HOURS`] used most
 * recently, and none once nothing is wanted (a tooltip closed). A new track starts
 * afresh. A read that failed is tried again when `attempt` changes. Returns only
 * the values `wanted` asks for, keyed by `nameKey`; one the recording does not give
 * at its hour is absent, so the caller shows its fallback. */
export function useCountryFactsAt<V>(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean,
  spec: FactSpec<V>, maxHours = MAX_NAME_HOURS, attempt = 0): ReadonlyMap<string, V> {
  const { held, key, version } = useFactStore(client, track, wanted, active, spec, maxHours, attempt);
  // Only what is wanted is handed out, so an update never copies everything read.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  return useMemo(() => held.pick(wanted), [held, key, version]);
}

/** Like `useCountryFactsAt`, but distinguishes pending (absent), failed
 * (`FAILED_FACT`), and completed without a value (null). A new `attempt` retries
 * failures at once; new content discards every dependent answer. */
export function useCountryFactsRead<V>(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean,
  spec: FactSpec<V>, maxHours = MAX_NAME_HOURS, attempt = 0): ReadonlyMap<string, V | null | typeof FAILED_FACT> {
  const { requests, key, version } = useFactStore(client, track, wanted, active, spec, maxHours, attempt);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  return useMemo(() => requests.pickRead(wanted), [requests, key, version]);
}

/** The store behind both hooks. */
function useFactStore<V>(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean,
  spec: FactSpec<V>, maxHours: number, attempt: number) {
  // Structured, so no tag (a mod's tags may hold any character) can blur two requests.
  const content = useContentRevision(client);
  const trackKey = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, spec.product, spec.identity, content]) : '';
  // The same countries in another order are the same request (`factsKey`).
  const key = factsKey(trackKey, wanted);
  const [version, setVersion] = useState(0);
  // One store and one set of reads per track and client; a change of what is wanted
  // asks only for what is new and cancels nothing (`FactRequests`).
  const store = useRef<{ trackKey: string; client: DashboardClient; store: HourStore<V>; requests: FactRequests<V> } | null>(null);
  if (!store.current || store.current.trackKey !== trackKey || store.current.client !== client) {
    store.current?.requests.cancel();
    const held = new HourStore<V>(MAX_NAME_HOURS);
    const source = track ? { sourceId: track.source.id, revision: track.source.revision, branchId: track.branch.id } : null;
    const read: FactRead<V> = (plan, signal, onChunk) => source ? readFacts(client, source, spec, plan, signal, onChunk) : Promise.resolve();
    store.current = { trackKey, client, store: held, requests: new FactRequests(held, read, () => setVersion(value => value + 1)) };
  }
  const { store: held, requests } = store.current;
  const attempted = useRef(attempt);
  // Reads stop when the widget goes away or another track takes over.
  useEffect(() => () => requests.cancel(), [requests]);
  useEffect(() => {
    // Off screen, nothing is read.
    if (!active || !track) { requests.cancel(); return; }
    // Nothing wanted: whatever was read is released.
    if (!wanted.length) { requests.cancel(); if (held.size) { held.clear(); setVersion(value => value + 1); } return; }
    requests.want(wanted, maxHours, spec.perRequest);
    // A new attempt tries a failing read again without waiting.
    if (attempted.current !== attempt) { attempted.current = attempt; requests.retry(); }
    // The key names every request; the arrays behind it are rebuilt each render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, active, attempt, requests]);
  return { held, requests, key, version };
}

/** Names from `country-styles`, which names at most 64 countries per request. */
const NAMES: FactSpec<string> = {
  product: 'country-styles', perRequest: 64,
  params: (branchId, hour, tags) => ({ branchId, hour, tags }),
  extract: data => new Map((data as CountryStyleReply).styles.flatMap(style => typeof style.name === 'string' ? [[style.tag, style.name] as const] : [])),
};

/** Country names as recorded at the given hours (`country-styles`), for labels
 * dated to an event; see `useCountryFactsAt`. */
export function useCountryNamesAt(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean,
  maxHours = MAX_NAME_HOURS, attempt = 0): ReadonlyMap<string, string> {
  return useCountryFactsAt(client, track, wanted, active, NAMES, maxHours, attempt);
}

/** Name reads retain pending, failed and completed-missing states for dated labels. */
export function useCountryNamesRead(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean,
  maxHours = MAX_NAME_HOURS, attempt = 0): ReadonlyMap<string, string | null | typeof FAILED_FACT> {
  return useCountryFactsRead(client, track, wanted, active, NAMES, maxHours, attempt);
}
