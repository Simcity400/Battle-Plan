import { useEffect, useState } from 'react';
import type { CountryStyle, CountryStyleReply } from '../generated/dashboard';
import type { DashboardClient } from './client';
import type { BoundTrack } from './model';
import { useProduct } from './products';

/** Names and map colours at monthly playback steps, held while the next request
 * loads so lines do not flash to fallback colours. Hover labels use their exact hour. */
export function useCountryLineStyles(client: DashboardClient, track: BoundTrack | undefined, tags: string[], at: number, active: boolean) {
  const step = 30 * 24;
  const hour = track ? Math.min(track.branch.lastHour, track.branch.firstHour + Math.floor((at - track.branch.firstHour) / step) * step) : 0;
  const request = track && tags.length ? { sourceId: track.source.id, revision: track.source.revision, product: 'country-styles',
    params: { branchId: track.branch.id, hour, tags } } : null;
  const styles = useProduct<CountryStyleReply>(client, request, active);
  const [known, setKnown] = useState<ReadonlyMap<string, CountryStyle>>(new Map());
  useEffect(() => setKnown(new Map()), [track?.source.id, track?.source.revision, track?.branch.id]);
  useEffect(() => {
    if (styles.data) setKnown(previous => new Map([...previous, ...styles.data!.styles.map(style => [style.tag, style] as const)]));
  }, [styles.data]);
  return { known, data: styles.data };
}
