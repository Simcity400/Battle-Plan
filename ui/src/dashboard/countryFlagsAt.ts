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

/** Country flags as preparation chose them at the given hours, as image URLs keyed by
 * `nameKey` (`useCountryFlagsRead`). A flag the recording does not give at its hour, or not
 * yet drawn, is absent: nothing shows in its place. */
export function useCountryFlagsAt(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean,
  size: CountryFlagSize = 'panel'): ReadonlyMap<string, string> {
  const flags = useCountryFlagsRead(client, track, wanted, active, size);
  return useMemo(() => {
    const urls = new Map<string, string>();
    for (const [key, flag] of flags) if (flag && !isFailedFact(flag) && flag.url) urls.set(key, flag.url);
    return urls;
  }, [flags]);
}

/** A country as a tooltip row shows it: its name and flag as recorded at an hour.
 * `known` is absent while pending; false when the name is missing or its read
 * failed. `nameFailed` distinguishes a failed read from a completed missing name. */
export type { CountryLabel } from './countryLabelData';

/** Countries named and flagged as the recording showed them at their hours (such as
 * a trade partner at the read behind a tooltip), never a later identity. Only the
 * countries `wanted` are asked for; until their names arrive the title says it is
 * loading, and where the recording has none it says unavailable. Every hour wanted
 * is read unless the caller bounds them with `maxHours`. `revision`
 * changes as names and flags arrive, for a chart to redraw. */
export function useCountryLabelsAt(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean,
  maxHours = Infinity) {
  const names = useCountryNamesAt(client, track, wanted, active, maxHours);
  const flags = useCountryFlagsRead(client, track, wanted, active);
  const first = track?.branch.firstHour, last = track?.branch.lastHour;
  const index = useMemo(() => new CountryLabelIndex(active && first !== undefined && last !== undefined ? [first, last] : null),
    [client, track?.source?.id, track?.source?.revision, track?.branch.id, active, first, last]);
  const labelAt = useMemo(() => index.at(names, flags), [index, names, flags]);
  const version = useRef({ labelAt, revision: 0 });
  if (version.current.labelAt !== labelAt) version.current = { labelAt, revision: version.current.revision + 1 };
  return { labelAt, revision: version.current.revision };
}
