import { failureText } from './queryError.ts';
import { useCallback, useMemo } from 'react';
import type { DashboardClient } from './client';
import type { BoundTrack } from './model';
import type { FactSpec } from './countryFacts';
import { useCountryFactsRead } from './countryNamesAt.ts';
import { stateNameKey, stateNameRequestOptions, validateStateNamesReply } from './stateNamesData.ts';
import type { StateAt, StateLabel } from './stateNamesData';

// State IDs use the same bounded per-hour request store as country facts.
const NAMES: FactSpec<StateLabel> = {
  product: 'place-names', perRequest: stateNameRequestOptions.perRequest, shared: true,
  params: (branchId, hour, states) => ({ branchId, places: [], states: states.map(state => ({ stateId: Number(state), hour })) }),
  extract: data => new Map(validateStateNamesReply(data).states.map(name => [String(name.stateId), { state: name.stateId, name: name.name, missing: name.missing }])),
};
export function useStateNamesAt(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly StateAt[], active: boolean) {
  const requests = useMemo(() => wanted.map(({ hour, state }) => ({ hour, tag: String(state) })), [wanted]);
  const names = useCountryFactsRead(client, track, requests, active, NAMES, stateNameRequestOptions.maxHours);
  const requested = useMemo(() => new Set(wanted.map(item => stateNameKey(item.hour, item.state))), [wanted]);
  return useCallback((hour: number, state: number): StateLabel => {
    const key = stateNameKey(hour, state), found = names.get(key);
    const read = found === undefined && (!active || !track || !requested.has(key)) ? null : found;
    return read instanceof Error ? { state, name: failureText(read, 'Couldn’t load state name.'), missing: failureText(read, 'Couldn’t load state name.'), failure: read }
      : read ?? { state, name: null, missing: read === undefined ? 'Loading state name…' : 'State name not recorded' };
  }, [names, requested, active, track]);
}
