import { useCallback, useMemo } from 'react';
import type { CountryFlagSize } from '../generated/dashboard';
import type { DashboardClient } from './client';
import { FAILED_FACT } from './countryFacts.ts';
import type { NameAt } from './countryFacts';
import { nameKey, useCountryNamesRead } from './countryNamesAt.ts';
import { useCountryFlagsRead } from './countryFlags.ts';
import type { FlagRead } from './countryFlags';
import type { BoundTrack } from './model';

/** Preserve the supplying read's pending, missing and failure states. */
export function flagReadMetadata(read: FlagRead | null | undefined | typeof FAILED_FACT): { flag?: string; flagPending: boolean; flagReason?: string } {
  return { flag: read && read !== FAILED_FACT ? read.url : undefined, flagPending: read === undefined,
    flagReason: read === FAILED_FACT ? 'The flag request failed.' : read?.reason ?? (read === null ? 'No readable flag was returned for this country at this hour.' : undefined) };
}

/** Country flags as preparation chose them at the given hours, as image URLs keyed by
 * `nameKey` (`useCountryFlagsRead`). A flag the recording does not give at its hour, or not
 * yet drawn, is absent: nothing shows in its place. */
export function useCountryFlagsAt(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean,
  size: CountryFlagSize = 'panel'): ReadonlyMap<string, string> {
  const flags = useCountryFlagsRead(client, track, wanted, active, size);
  return useMemo(() => {
    const urls = new Map<string, string>();
    for (const [key, flag] of flags) if (flag && flag !== FAILED_FACT && flag.url) urls.set(key, flag.url);
    return urls;
  }, [flags]);
}

/** A country as a tooltip row shows it: its name and flag as recorded at an hour.
 * `known` is absent while pending; false when the name is missing or its read
 * failed. `nameFailed` distinguishes a failed read from a completed missing name. */
export type CountryLabel = (tag: string) => { name: string; flag?: string; known?: boolean; nameFailed?: boolean; flagPending?: boolean; flagReason?: string };

/** Countries named and flagged as the recording showed them at their hours (such as
 * a trade partner at the read behind a tooltip), never a later identity. Only the
 * countries `wanted` are asked for; until their names arrive the title says it is
 * loading, and where the recording has none it says unavailable. Every hour wanted
 * is read unless the caller bounds them with `maxHours`. `revision`
 * changes as names and flags arrive, for a chart to redraw. */
export function useCountryLabelsAt(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean,
  maxHours = Infinity) {
  const names = useCountryNamesRead(client, track, wanted, active, maxHours);
  const flags = useCountryFlagsRead(client, track, wanted, active);
  const labelAt = useCallback((hour: number): CountryLabel => tag => {
    const name = names.get(nameKey(hour, tag));
    const flag = flags.get(nameKey(hour, tag));
    return { name: typeof name === 'string' ? name : name === undefined ? 'Loading name…' : 'Country name unavailable', ...flagReadMetadata(flag),
      known: name === undefined ? undefined : typeof name === 'string', nameFailed: name === FAILED_FACT };
  }, [names, flags]);
  return { labelAt, revision: names.size + flags.size };
}
