import { useEffect, useState } from 'react';
import type { CountryStyleReply } from '../generated/dashboard';
import type { DashboardClient } from './client';
import type { BoundTrack } from './model';
import { loadProduct } from './products';

/** Most distinct hours one label set may ask names for. */
export const MAX_NAME_HOURS = 64;
/** Most countries one `country-styles` request names. */
const TAGS_PER_REQUEST = 64;

/** A country's name as the game showed it at an hour. */
export interface NameAt { hour: number; tag: string }

export const nameKey = (hour: number, tag: string) => JSON.stringify([hour, tag]);

/** Country names as recorded at the given hours (`country-styles`, one request
 * per hour and at most MAX_NAME_HOURS hours), for labels dated to an event. A
 * name the recording does not give at its hour is simply absent: the caller
 * falls back to another name. */
export function useCountryNamesAt(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean): ReadonlyMap<string, string> {
  const byHour = new Map<number, Set<string>>();
  for (const item of wanted) {
    const tags = byHour.get(item.hour) ?? new Set<string>();
    tags.add(item.tag);
    byHour.set(item.hour, tags);
  }
  const hours = [...byHour.keys()].sort((a, b) => a - b).slice(0, MAX_NAME_HOURS);
  // Structured, so no tag (a mod's tags may hold any character) can blur two requests.
  const key = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, hours.map(hour => [hour, [...byHour.get(hour)!].sort()])]) : '';
  const [names, setNames] = useState<{ key: string; names: ReadonlyMap<string, string> }>({ key: '', names: new Map() });
  useEffect(() => {
    if (!active || !track || !hours.length) return;
    const controller = new AbortController();
    void (async () => {
      const found = new Map<string, string>();
      for (const hour of hours) {
        const tags = [...byHour.get(hour)!];
        for (let start = 0; start < tags.length; start += TAGS_PER_REQUEST) {
          try {
            const reply = await loadProduct<CountryStyleReply>(client, { sourceId: track.source.id, revision: track.source.revision, product: 'country-styles',
              params: { branchId: track.branch.id, hour, tags: tags.slice(start, start + TAGS_PER_REQUEST) } }, controller.signal);
            for (const style of reply.styles) if (style.name) found.set(nameKey(hour, style.tag), style.name);
          } catch {
            // An hour the recording cannot name at (a gap) keeps the fallback.
          }
          if (controller.signal.aborted) return;
        }
      }
      setNames({ key, names: found });
    })();
    return () => controller.abort();
    // The key names every request; the arrays behind it are rebuilt each render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, key, active]);
  return names.key === key ? names.names : EMPTY;
}

const EMPTY: ReadonlyMap<string, string> = new Map();
