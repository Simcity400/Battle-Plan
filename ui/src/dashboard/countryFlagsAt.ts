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
import type { CountryLabel, CountryLabelValue } from './countryLabelData';

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
  const read = useMemo(() => index.at(names, flags), [index, names, flags]);
  // The last name and flag read for each country: a country whose label at a new hour is still being
  // read shows them, so a known country never shows a placeholder while playback moves on.
  const known = useMemo(() => new Map<string, { name?: string; flag?: string; held: Map<CountryLabelValue, { name?: string; flag?: string; label: CountryLabelValue }> }>(), [index]);
  const labelAt = useMemo(() => (hour: number): CountryLabel => {
    const at = read(hour);
    return tag => {
      const label = at(tag);
      let last = known.get(tag);
      if (!last) known.set(tag, last = { held: new Map() });
      if (label.known === true) last.name = label.name;
      if (label.flag) last.flag = label.flag;
      const name = label.known === undefined && !label.nameFailed ? last.name : undefined;
      const flag = label.flagPending ? last.flag : undefined;
      if (name === undefined && flag === undefined) return label;
      const before = last.held.get(label);
      if (before && before.name === name && before.flag === flag) return before.label;
      const held = { ...label, ...(name !== undefined ? { name, known: true } : {}), ...(flag !== undefined ? { flag, flagPending: false } : {}) };
      last.held.set(label, { name, flag, label: held });
      return held;
    };
  }, [read, known]);
  const version = useRef({ labelAt, revision: 0 });
  if (version.current.labelAt !== labelAt) version.current = { labelAt, revision: version.current.revision + 1 };
  return { labelAt, revision: version.current.revision };
}
