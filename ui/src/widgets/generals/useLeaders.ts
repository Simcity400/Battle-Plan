import { useMemo } from 'react';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { useCountriesSelectionAt } from '../../dashboard/useCountriesSelectionAt';
import { useProduct } from '../../dashboard/products';
import type { LeaderKinds, LeaderSelection, LeaderView } from '../../generated/dashboard';
import { leaderRequestSelection, validateUnitLeadersReply } from './generalsData';
import type { UnitLeadersData } from './generalsData';

export function useLeaderCountries(widget: WidgetSpec, runtime: WidgetRuntime, hour: number, active: boolean) {
  const selected = useCountriesSelectionAt(widget, runtime, hour, active);
  const requestSelection = useMemo(() => leaderRequestSelection(selected.requestTags), [selected.requestTags]);
  return { ...selected, requestSelection };
}
export function useLeaders(runtime: WidgetRuntime, trackId: string, selection: LeaderSelection | undefined, view: LeaderView, active: boolean, leaders: LeaderKinds = 'all') {
  const track = runtime.tracks.find(t => t.id === trackId);
  const request = track && selection ? { sourceId: track.source.id, revision: track.source.revision, product: 'unit-leaders',
    params: { branchId: track.branch.id, selection, view, leaders } } : null;
  return useProduct<UnitLeadersData>(runtime.client, request, active, validateUnitLeadersReply);
}
