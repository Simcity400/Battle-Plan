import { useMemo, useRef } from 'react';
import type { CountryFlagSize } from '../generated/dashboard';
import type { DashboardClient } from './client';
import { isFailedFact } from './countryFacts.ts';
import { CountryLabelIndex } from './countryLabelData.ts';
import type { NameAt } from './countryFacts';
import { useCountryNamesAt } from './countryNamesAt.ts';
import { useCountryFlagsRead } from './countryFlags.ts';
import type { BoundTrack } from './model';

export { flagReadMetadata } from './countryLabelData.ts';

export function useCountryFlagsAt(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean,
  size: CountryFlagSize = 'panel'): ReadonlyMap<string, string> {
  const flags = useCountryFlagsRead(client, track, wanted, active, size);
  return useMemo(() => {
    const urls = new Map<string, string>();
    for (const [key, flag] of flags) if (flag && !isFailedFact(flag) && flag.url) urls.set(key, flag.url);
    return urls;
  }, [flags]);
}

export type { CountryLabel } from './countryLabelData';

export function useCountryLabelsAt(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean,
  maxHours = Infinity) {
  const names = useCountryNamesAt(client, track, wanted, active, maxHours);
  const flags = useCountryFlagsRead(client, track, wanted, active, 'panel');
  const first = track?.branch.firstHour, last = track?.branch.lastHour;
  const index = useMemo(() => new CountryLabelIndex(active && first !== undefined && last !== undefined ? [first, last] : null),
    [client, track?.source?.id, track?.source?.revision, track?.branch.id, active, first, last]);
  const labelAt = useMemo(() => index.at(names, flags), [index, names, flags]);
  const version = useRef({ labelAt, revision: 0 });
  if (version.current.labelAt !== labelAt) version.current = { labelAt, revision: version.current.revision + 1 };
  return { labelAt, exactAt: labelAt, revision: version.current.revision };
}
