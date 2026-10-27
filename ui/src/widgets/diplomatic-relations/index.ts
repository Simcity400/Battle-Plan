import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { setToken } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import type { ScoreboardPreparedReply } from '../../generated/dashboard';
import { scoreboardRegistryRequest, validatePreparedScoreboard } from '../scoreboard/preparedScoreboard';
import DiplomaticRelationsWidget from './DiplomaticRelationsWidget';

/** A top N ranks by any recorded Scoreboard measure. */
function useMeasures(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'scoreboard', params: scoreboardRegistryRequest(track.branch.id) } : null;
  const registry = useProduct<ScoreboardPreparedReply>(runtime.client, request, true, validatePreparedScoreboard, track?.source).data?.data.columns ?? [];
  return { topMeasure: registry.filter(column => column.available).map(column => ({ value: setToken(column.key), label: column.label })) };
}

export default { render: DiplomaticRelationsWidget, useOptionChoices: useMeasures } satisfies WidgetRenderer;
