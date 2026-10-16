import { useEffect, useMemo, useState } from 'react';
import type { CountryStyle, CountryStyleReply } from '../generated/dashboard';
import type { DashboardClient } from './client';
import type { BoundTrack } from './model';
import { useProducts } from './products';
import { STYLES } from './useCountriesAt';

/** Names and map colours at monthly playback steps, held while the next request
 * loads so lines do not flash to fallback colours. Hover labels use their exact hour. */
export function useCountryLineStyles(client: DashboardClient, track: BoundTrack | undefined, tags: string[], at: number, active: boolean) {
  const step = 30 * 24;
  const hour = track ? Math.min(track.branch.lastHour, track.branch.firstHour + Math.floor((at - track.branch.firstHour) / step) * step) : 0;
  // This is the native product's per-request scheduling unit, never a country
  // selection limit. Canonical chunks share cached replies when a list reorders.
  const wanted = [...new Set(tags)].sort();
  const requests = [];
  if (track) for (let index = 0; index < wanted.length; index += STYLES.perRequest) {
    requests.push({ sourceId: track.source.id, revision: track.source.revision, product: STYLES.product,
      params: STYLES.params(track.branch.id, hour, wanted.slice(index, index + STYLES.perRequest)) });
  }
  const styles = useProducts<CountryStyleReply>(client, requests, active);
  const data = useMemo<CountryStyleReply | undefined>(() => styles.data?.length ? {
    branchId: styles.data[0].branchId, hour: styles.data[0].hour,
    styles: styles.data.flatMap(reply => reply.styles),
    warnings: [...new Set(styles.data.flatMap(reply => reply.warnings))],
  } : undefined, [styles.data]);
  const [known, setKnown] = useState<ReadonlyMap<string, CountryStyle>>(new Map());
  useEffect(() => setKnown(new Map()), [track?.source.id, track?.source.revision, track?.branch.id]);
  useEffect(() => {
    if (data) setKnown(previous => new Map([...previous, ...data.styles.map(style => [style.tag, style] as const)]));
  }, [data]);
  return { known, data };
}
