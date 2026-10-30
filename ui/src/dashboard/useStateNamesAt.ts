import { useCallback, useMemo } from 'react';
import type { DashboardClient } from './client';
import type { BoundTrack } from './model';
import type { FactSpec } from './countryFacts';
import { useCountryFactsAt } from './countryNamesAt';
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
  const names = useCountryFactsAt(client, track, requests, active, NAMES, stateNameRequestOptions.maxHours);
  return useCallback((hour: number, state: number) => names.get(stateNameKey(hour, state)) ?? { state, name: null, missing: 'State name not recorded' }, [names]);
}
