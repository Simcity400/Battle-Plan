import { metadataCountry } from './countryKey.ts';
import { useEffect, useMemo, useRef, useState } from 'react';
import { nameKey } from './countryFacts.ts';
import type { NameAt } from './countryFacts';
import { useCountryNamesAt } from './countryNamesAt.ts';
import type { DashboardClient } from './client';
import { nameReadText, nameReadFailure } from './nameReads.ts';
import { ideaTooltipSources } from './ideaTooltip.ts';
import type { BoundTrack } from './model';
import type { SpendIdeaTooltips } from '../generated/dashboard';

const RETRY_MS = 10_000;

export function useIdeaTooltips(data: SpendIdeaTooltips | undefined, track: Pick<BoundTrack, 'source' | 'branch'> | undefined,
  client: DashboardClient, active: boolean, fallbackName?: (tag: string) => string) {
  const [requested, setRequested] = useState<NameAt[]>([]);
  const asked = useRef(new Map<string, number>());
  const [attempt, setAttempt] = useState(0);
  useEffect(() => { setRequested([]); asked.current.clear(); }, [data, track?.source.id, track?.source.revision, track?.branch.id]);
  const names = useCountryNamesAt(client, track, requested, active, Infinity, attempt);
  return useMemo(() => {
    if (!data || !track) return null;
    const metadataName = fallbackName ?? ((tag: string) => metadataCountry(track.source.metadata.countries, tag)?.name ?? 'Country name unavailable');
    const nameAt = (tag: string, hour: number) => {
      const key = nameKey(hour, tag);
      const read = names.get(key);
      if (read?.kind === 'value') return read.value;
      const last = asked.current.get(key);
      const now = performance.now();
      if (active && last == null) {
        asked.current.set(key, now);
        queueMicrotask(() => setRequested(list => [...list, { hour, tag }]));
      } else if (active && read?.kind === 'failed' && last != null && now - last > RETRY_MS) {
        asked.current.set(key, now);
        queueMicrotask(() => setAttempt(value => value + 1));
      }
      return read === undefined ? fallbackName?.(tag) ?? nameReadText(read) : nameReadText(read);
    };
    return {
      sources: { ...ideaTooltipSources(data, metadataName, nameAt), failureAt: (tag: string, hour: number) => nameReadFailure(names.get(nameKey(hour, tag))) },
      infos: new Map(data.ideas.map(info => [info.token, info])),
      art: new Map(data.artwork.map(item => [item.token, item])),
    };
  }, [data, track, names, active, fallbackName]);
}
