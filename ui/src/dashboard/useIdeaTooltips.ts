import { useEffect, useMemo, useRef, useState } from 'react';
import type { IdeaHistoryReply } from '../generated/dashboard';
import { nameKey } from './countryFacts.ts';
import type { NameAt } from './countryFacts';
import { useCountryNamesAt } from './countryNamesAt.ts';
import type { DashboardClient } from './client';
import { ideaTooltipSources } from './ideaTooltip.ts';
import type { BoundTrack } from './model';

/** What the idea widgets' tooltips read: the formats and texts of the reply, each
 * idea and its artwork, and the countries targeted modifiers name, as they were
 * when each idea was taken on (else their name in the recording's metadata).
 * Those names are read on demand, only for the tooltips actually built, each hour
 * once, so however long the campaign, every name shown can be read. What is asked
 * for is bounded by the reply (its ideas' targets at their hours), not by time, and
 * is dropped with the reply; nothing asked is ever dropped while it is still shown,
 * so asking never repeats itself. */
/** How long a missing name waits before it is asked for again. */
const RETRY_MS = 10_000;

export function useIdeaTooltips(data: Pick<IdeaHistoryReply, 'ideas' | 'artwork' | 'modifiers' | 'texts'> | undefined, track: Pick<BoundTrack, 'source' | 'branch'> | undefined,
  client: DashboardClient, active: boolean, fallbackName?: (tag: string) => string) {
  const [requested, setRequested] = useState<NameAt[]>([]);
  // When each name was last asked for, so one still missing is asked again, at most every few seconds.
  const asked = useRef(new Map<string, number>());
  const [attempt, setAttempt] = useState(0);
  useEffect(() => { setRequested([]); asked.current.clear(); }, [data, track?.source.id, track?.source.revision, track?.branch.id]);
  // Asked for only as tooltips are built, so no cap on hours is needed; the store keeps every hour asked.
  const names = useCountryNamesAt(client, track, requested, active, Infinity, attempt);
  return useMemo(() => {
    if (!data || !track) return null;
    const metadataName = fallbackName ?? ((tag: string) => track.source.metadata.countries.find(country => country.tag === tag)?.name ?? 'Country name unavailable');
    const nameAt = (tag: string, hour: number) => {
      const key = nameKey(hour, tag);
      const found = names.get(key);
      if (typeof found === 'string') return found;
      // Asked for only while the widget is shown, after this render: the tooltip shows
      // the fallback until the name arrives. One still missing (its read failed) is
      // asked again when shown again, at most every RETRY_MS.
      const last = asked.current.get(key);
      const now = performance.now();
      if (active && last == null) {
        asked.current.set(key, now);
        queueMicrotask(() => setRequested(list => [...list, { hour, tag }]));
      } else if (active && last != null && now - last > RETRY_MS) {
        asked.current.set(key, now);
        queueMicrotask(() => setAttempt(value => value + 1));
      }
      return metadataName(tag);
    };
    return {
      sources: ideaTooltipSources(data, metadataName, nameAt),
      infos: new Map(data.ideas.map(info => [info.token, info])),
      art: new Map(data.artwork.map(item => [item.token, item])),
    };
  }, [data, track, names, active, fallbackName]);
}
