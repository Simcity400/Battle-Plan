import { useEffect, useMemo, useRef, useState } from 'react';
import type { CountryStyleReply } from '../generated/dashboard';
import type { DashboardClient } from './client';
import { HourStore, MAX_NAME_HOURS, nameKey, planRequests, readFacts, wantedHours } from './countryFacts.ts';
import type { FactSpec, NameAt } from './countryFacts';
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
  // Structured, so no tag (a mod's tags may hold any character) can blur two requests.
  const trackKey = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, spec.product]) : '';
  const key = JSON.stringify([trackKey, wanted.map(item => [item.hour, item.tag])]);
  const store = useRef<{ trackKey: string; store: HourStore<V> } | null>(null);
  if (!store.current || store.current.trackKey !== trackKey) store.current = { trackKey, store: new HourStore<V>(MAX_NAME_HOURS) };
  const held = store.current.store;
  const [version, setVersion] = useState(0);
  useEffect(() => {
    if (!active || !track) return;
    // Nothing wanted: whatever was read is released.
    if (!wanted.length) { if (held.size) { held.clear(); setVersion(value => value + 1); } return; }
    held.want(wantedHours(wanted, maxHours));
    const plan = planRequests(held, wanted, maxHours, spec.perRequest);
    if (!plan.length) return;
    const controller = new AbortController();
    void readFacts(client, { sourceId: track.source.id, revision: track.source.revision, branchId: track.branch.id }, spec, plan, controller.signal,
      (hour, tags, found) => {
        if (found) held.record(hour, tags, found);
        setVersion(value => value + 1);
      });
    return () => controller.abort();
    // The key names every request; the arrays behind it are rebuilt each render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, key, active, attempt, held]);
  // Only what is wanted is handed out, so an update never copies everything read.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  return useMemo(() => held.pick(wanted), [held, key, version]);
}

/** Names from `country-styles`, which names at most 64 countries per request. */
const NAMES: FactSpec<string> = {
  product: 'country-styles', perRequest: 64,
  params: (branchId, hour, tags) => ({ branchId, hour, tags }),
  extract: data => new Map((data as CountryStyleReply).styles.flatMap(style => style.name ? [[style.tag, style.name] as const] : [])),
};

/** Country names as recorded at the given hours (`country-styles`), for labels
 * dated to an event; see `useCountryFactsAt`. */
export function useCountryNamesAt(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean,
  maxHours = MAX_NAME_HOURS, attempt = 0): ReadonlyMap<string, string> {
  return useCountryFactsAt(client, track, wanted, active, NAMES, maxHours, attempt);
}
