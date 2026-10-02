import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import type { ConstructionLogReply } from '../../generated/dashboard';
import { buildingChoices, validateConstructionLogReply } from './constructionData';

/** The widget's campaign branch and its construction log (every country's projects). */
export function useConstructionLog(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'construction-log',
    params: { branchId: track.branch.id } } : null;
  return { track, ...useProduct<ConstructionLogReply>(runtime.client, request, active, validateConstructionLogReply) };
}

/** The Building types setting's choices: the types this campaign's projects name, as
 * the game names and pictures them (never a fixed list). */
export function useConstructionChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useConstructionLog(widget, runtime, true);
  return { types: buildingChoices(data) };
}
