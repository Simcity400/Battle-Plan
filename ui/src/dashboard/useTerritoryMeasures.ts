import type { WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { measureChoices } from './territoryMeasure';
import { useTerritoryMetadata } from '../widgets/territory/preparedTerritory';

/** Territory availability is shared with every widget using its adapter. */
export function useTerritoryMeasures(widget: WidgetSpec, runtime: WidgetRuntime, enabled = true) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const { data } = useTerritoryMetadata(runtime.client,enabled ? track : undefined,true);
  return measureChoices(data);
}
