import type { OptionChoices, WidgetProps, WidgetRenderer } from '../../dashboard/definitions';
import { setToken } from '../../dashboard/options';
import { narrowTableOf, tableSortChoices } from '../../charts/table/options';
import { SCORE_LABELS, SCORE_PARTS } from './gameScoreData';
import { createElement } from 'react';
import GameScoreWidget from './GameScoreWidget';
import CanvasGameScore from './CanvasGameScore';
import CanvasGameScoreRace from './CanvasGameScoreRace';
import { LIVE_WIDGETS } from '../../live/liveSwitch.ts';

const COLUMNS: OptionChoices = {
  tableSort: tableSortChoices((['country', ...SCORE_PARTS, 'multiplier'] as const).map(key => ({ key, label: SCORE_LABELS[key] }))),
  topMeasure: SCORE_PARTS.map(part => ({ value: setToken(part), label: SCORE_LABELS[part] })),
};
/** The canvas table or race where they cover the settings: every country, following playback. */
function GameScore(props: WidgetProps) {
  const live = LIVE_WIDGETS && props.widget.time.mode === 'follow' && props.widget.countries.kind === 'all';
  const race = props.widget.options?.view === 'race';
  // The canvas table hides columns that do not fit; "Scroll sideways" keeps the DOM table.
  const table = live && !race && props.widget.options?.factions !== true && narrowTableOf(props.widget.options) === 'hide';
  return createElement(live && race ? CanvasGameScoreRace : table ? CanvasGameScore : GameScoreWidget, props);
}

export default { render: GameScore, useOptionChoices: () => COLUMNS } satisfies WidgetRenderer;
