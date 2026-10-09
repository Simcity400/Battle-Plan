import type { WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { useProduct } from './products';
import { measureChoices, validateTerritoryReply } from './territoryMeasure';
import type { TerritoryReply } from '../generated/dashboard';

/** Territory availability is shared with every widget using its adapter. */
export function useTerritoryMeasures(widget: WidgetSpec, runtime: WidgetRuntime, enabled = true) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track && enabled ? { sourceId: track.source.id, revision: track.source.revision, product: 'territory',
    params: { branchId: track.branch.id, tags: [], top: null } } : null;
  const { data } = useProduct<TerritoryReply>(runtime.client, request, true, validateTerritoryReply);
  return measureChoices(data);
}
