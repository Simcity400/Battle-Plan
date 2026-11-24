import { useMemo, useRef } from 'react';
import type { DashboardClient } from './client';
import { CountryLabelIndex } from './countryLabelData.ts';
import type { NameAt } from './countryFacts';
import { useCountryNamesAt } from './countryNamesAt.ts';
import { useCountryFlagsRead } from './countryFlags.ts';
import type { BoundTrack } from './model';

export { flagReadMetadata } from './countryLabelData.ts';

export type { CountryLabel } from './countryLabelData';
import type { CountryLabel, CountryLabelValue } from './countryLabelData';

export function useCountryLabelsAt(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean,
  maxHours = Infinity, exact = false) {
  const names = useCountryNamesAt(client, track, wanted, active, maxHours, 0, exact);
  const flags = useCountryFlagsRead(client, track, wanted, active, 'panel', exact);
  const first = track?.branch.firstHour, last = track?.branch.lastHour;
  const index = useMemo(() => new CountryLabelIndex(active && first !== undefined && last !== undefined ? [first, last] : null),
    [client, track?.source?.id, track?.source?.revision, track?.branch.id, active, first, last]);
  const read = useMemo(() => index.at(names, flags), [index, names, flags]);
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
  return { labelAt, exactAt: read, revision: version.current.revision };
}
