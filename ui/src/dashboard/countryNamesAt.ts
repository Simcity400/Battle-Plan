import { useEffect, useRef, useState } from 'react';
import type { CountryStyleReply } from '../generated/dashboard';
import type { DashboardClient } from './client';
import type { BoundTrack } from './model';
import { loadProduct } from './products.ts';

/** Most distinct hours one label set may ask names for. */
export const MAX_NAME_HOURS = 64;
/** Most countries one `country-styles` request names. */
const TAGS_PER_REQUEST = 64;

/** A country's name as the game showed it at an hour. */
export interface NameAt { hour: number; tag: string }

export const nameKey = (hour: number, tag: string) => JSON.stringify([hour, tag]);

/** Country names as recorded at the given hours (`country-styles`, one request
 * per hour, at most `maxHours` hours), for labels dated to an event. Each hour and
 * country is read once per track; names read stay; a read that failed is tried
 * again when `attempt` changes. A name the recording does not give at its hour is
 * simply absent: the caller falls back to another name. */
export function useCountryNamesAt(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean,
  maxHours = MAX_NAME_HOURS, attempt = 0): ReadonlyMap<string, string> {
  const byHour = new Map<number, Set<string>>();
  for (const item of wanted) {
    const tags = byHour.get(item.hour) ?? new Set<string>();
    tags.add(item.tag);
    byHour.set(item.hour, tags);
  }
  const hours = [...byHour.keys()].sort((a, b) => a - b).slice(0, maxHours);
  // Structured, so no tag (a mod's tags may hold any character) can blur two requests.
  const trackKey = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id]) : '';
  const key = track ? JSON.stringify([trackKey, hours.map(hour => [hour, [...byHour.get(hour)!].sort()])]) : '';
  // Names already read stay available while more are read, for the same track.
  const [names, setNames] = useState<{ trackKey: string; names: ReadonlyMap<string, string> }>({ trackKey: '', names: new Map() });
  // The hours and countries already read for this track, so each is read once.
  const read = useRef<{ trackKey: string; keys: Set<string> }>({ trackKey: '', keys: new Set() });
  useEffect(() => {
    if (!active || !track || !hours.length) return;
    if (read.current.trackKey !== trackKey) read.current = { trackKey, keys: new Set() };
    const done = read.current.keys;
    const todo = hours.map(hour => [hour, [...byHour.get(hour)!].filter(tag => !done.has(nameKey(hour, tag)))] as const).filter(([, tags]) => tags.length);
    if (!todo.length) return;
    const controller = new AbortController();
    void (async () => {
      for (const [hour, tags] of todo) {
        for (let start = 0; start < tags.length; start += TAGS_PER_REQUEST) {
          const chunk = tags.slice(start, start + TAGS_PER_REQUEST);
          const found = new Map<string, string>();
          let answered = false;
          try {
            const reply = await loadProduct<CountryStyleReply>(client, { sourceId: track.source.id, revision: track.source.revision, product: 'country-styles',
              params: { branchId: track.branch.id, hour, tags: chunk } }, controller.signal);
            for (const style of reply.styles) if (style.name) found.set(nameKey(hour, style.tag), style.name);
            answered = true;
          } catch {
            // Not read (a gap, or a failure): the fallback shows, and it is asked again next time.
          }
          if (controller.signal.aborted) return;
          if (answered) for (const tag of chunk) done.add(nameKey(hour, tag));
          setNames(previous => ({ trackKey, names: previous.trackKey === trackKey ? new Map([...previous.names, ...found]) : found }));
        }
      }
    })();
    return () => controller.abort();
    // The key names every request; the arrays behind it are rebuilt each render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, key, active, attempt]);
  return names.trackKey === trackKey ? names.names : EMPTY;
}

const EMPTY: ReadonlyMap<string, string> = new Map();
