import type { OptionChoices, WidgetProps, WidgetRenderer } from '../../dashboard/definitions';
import { useProduct } from '../../dashboard/products';
import { setToken } from '../../dashboard/options';
import { narrowTableOf, tableSortChoices } from '../../charts/table/options';
import { FALLBACK_LABELS, SCORE_PARTS, gameScoreLabel, validateGameScoreReply } from './gameScoreData';
import type { GameScoreReply } from '../../generated/dashboard';
import { createElement } from 'react';
import GameScoreWidget from './GameScoreWidget';
import CanvasGameScore from './CanvasGameScore';
import CanvasGameScoreRace from './CanvasGameScoreRace';
import { LIVE_WIDGETS } from '../../live/liveSwitch.ts';
function useColumns(widget: WidgetProps['widget'], runtime: WidgetProps['runtime']): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'game_score', params: { branchId: track.branch.id } } : null;
  const { data } = useProduct<GameScoreReply>(runtime.client, request, true, validateGameScoreReply);
  const labels = data?.labels ?? FALLBACK_LABELS;
  return { tableSort: tableSortChoices(Object.keys(FALLBACK_LABELS).map(key => ({ key, label: gameScoreLabel(labels, key as keyof typeof FALLBACK_LABELS) }))),
    topMeasure: SCORE_PARTS.map(part => ({ value: setToken(part), label: gameScoreLabel(labels, part) })) };
}
/** The canvas table or race where they cover the settings: every country, following playback. */
function GameScore(props: WidgetProps) {
  const live = LIVE_WIDGETS && props.widget.time.mode === 'follow' && props.widget.countries.kind === 'all';
  const race = props.widget.options?.view === 'race';
  // The canvas table hides columns that do not fit; "Scroll sideways" keeps the DOM table.
  const table = live && !race && props.widget.options?.factions !== true && narrowTableOf(props.widget.options) === 'hide';
  return createElement(live && race ? CanvasGameScoreRace : table ? CanvasGameScore : GameScoreWidget, props);
}

export default { render: GameScore, useOptionChoices: useColumns } satisfies WidgetRenderer;
