import { useMemo } from 'react';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { useCountriesSelectionAt } from '../../dashboard/useCountriesSelectionAt';
import { useFollowedProduct, useProduct } from '../../dashboard/products';
import type { LeaderKinds, LeaderSelection, LeaderView } from '../../generated/dashboard';
import { leaderRequestSelection, validateUnitLeadersReply } from './generalsData';
import type { UnitLeadersData } from './generalsData';

export function useLeaderCountries(widget: WidgetSpec, runtime: WidgetRuntime, hour: number, active: boolean) {
  const selected = useCountriesSelectionAt(widget, runtime, hour, active);
  const requestSelection = useMemo(() => leaderRequestSelection(selected.requestTags), [selected.requestTags]);
  return { ...selected, requestSelection };
}
/** Leaders at `hour` follow playback: the running read lands and stays shown while the
 * newest hour is read; `answered` is the hour the shown reply is for. */
export function useLeaders(runtime: WidgetRuntime, trackId: string, selection: LeaderSelection | undefined, view: LeaderView, active: boolean, leaders: LeaderKinds = 'all', hour?: number) {
  const track = runtime.tracks.find(t => t.id === trackId);
  const request = track && selection ? { sourceId: track.source.id, revision: track.source.revision, product: 'unit-leaders',
    params: { branchId: track.branch.id, selection, view, leaders, ...(hour === undefined ? {} : { hour }) } } : null;
  const fixed = useProduct<UnitLeadersData>(runtime.client, hour === undefined ? request : null, active, validateUnitLeadersReply, track?.source);
  const followed = useFollowedProduct<UnitLeadersData>(runtime.client, hour === undefined ? null : request, ['hour'], active, validateUnitLeadersReply, track?.source);
  if (hour === undefined) return { ...fixed, answered: undefined };
  return { data: followed.data, error: followed.error, retry: followed.retry,
    answered: followed.data && followed.moment ? (JSON.parse(followed.moment) as [number])[0] : hour };
}
