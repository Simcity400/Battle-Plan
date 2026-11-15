import { useEffect, useMemo, useState } from 'react';
import type { CountryStyle, CountryStyleReply } from '../generated/dashboard';
import type { DashboardClient } from './client';
import { lookHour } from './countryLooks.ts';
import { useHeldFailure } from './heldFailures';
import type { BoundTrack } from './model';
import { useProducts } from './products';
import { STYLES } from './useCountriesAt';

/** Names and map colours at monthly playback steps within the recorded stretch shown
 * (`lookHour`: never an hour of a gap), held while the stretch's next request loads so
 * lines do not flash to fallback colours. Hover labels use their exact hour. */
export function useCountryLineStyles(client: DashboardClient, track: BoundTrack | undefined, tags: string[], at: number, active: boolean) {
  const sample = track ? lookHour(track.branch, Math.max(track.branch.firstHour, Math.min(track.branch.lastHour, at))) : null;
  const hour = sample?.hour ?? null, stretch = sample?.stretch ?? null;
  // Sorted and each once, so a reordered selection is the same cached request.
  const wanted = [...new Set(tags)].sort();
  const requests = track && hour !== null && wanted.length ? [{ sourceId: track.source.id, revision: track.source.revision, product: STYLES.product,
    params: STYLES.params(track.branch.id, hour, wanted) }] : [];
  const styles = useProducts<CountryStyleReply>(client, requests, active);
  const data = useMemo<CountryStyleReply | undefined>(() => styles.data?.length ? {
    branchId: styles.data[0].branchId, hour: styles.data[0].hour,
    styles: styles.data.flatMap(reply => reply.styles),
    warnings: [...new Set(styles.data.flatMap(reply => reply.warnings))],
  } : undefined, [styles.data]);
  const [known, setKnown] = useState<ReadonlyMap<string, CountryStyle>>(new Map());
  useEffect(() => setKnown(new Map()), [track?.source.id, track?.source.revision, track?.branch.id]);
  // Another country may hold a tag after a gap: a new recorded stretch reads its colours afresh.
  useEffect(() => { if (stretch !== null) setKnown(new Map()); }, [stretch]);
  useEffect(() => {
    if (data) setKnown(previous => new Map([...previous, ...data.styles.map(style => [style.tag, style] as const)]));
  }, [data]);
  // Held colours stay drawn while the frame reports a failed refresh with its retry.
  const error = useHeldFailure(styles.error, known.size > 0, styles.retry);
  return { known, data, error, retry: styles.retry };
}
