import type { OptionChoices, WidgetProps, WidgetRenderer } from '../../dashboard/definitions';
import { useProduct } from '../../dashboard/products';
import { setToken } from '../../dashboard/options';
import { tableSortChoices } from '../../charts/table/options';
import { FALLBACK_LABELS, SCORE_PARTS, gameScoreLabel, validateGameScoreReply } from './gameScoreData';
import type { GameScoreReply } from '../../generated/dashboard';
import GameScoreWidget from './GameScoreWidget';
function useColumns(widget: WidgetProps['widget'], runtime: WidgetProps['runtime']): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'game_score', params: { branchId: track.branch.id } } : null;
  const { data } = useProduct<GameScoreReply>(runtime.client, request, true, validateGameScoreReply);
  const labels = data?.labels ?? FALLBACK_LABELS;
  return { tableSort: tableSortChoices(Object.keys(FALLBACK_LABELS).map(key => ({ key, label: gameScoreLabel(labels, key as keyof typeof FALLBACK_LABELS) }))),
    topMeasure: SCORE_PARTS.map(part => ({ value: setToken(part), label: gameScoreLabel(labels, part) })) };
}
export default { render: GameScoreWidget, useOptionChoices: useColumns } satisfies WidgetRenderer;
