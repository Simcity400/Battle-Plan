import type { WidgetRenderer } from '../../dashboard/definitions';
import { tableSortChoices } from '../../charts/table/options';
import GameScoreWidget from './GameScoreWidget';
import { SCORE_LABELS, SCORE_PARTS } from './gameScoreData';

const CHOICES = { tableSort: tableSortChoices((['country', ...SCORE_PARTS] as const).map(key => ({ key, label: SCORE_LABELS[key] }))) };

export default { render: GameScoreWidget, useOptionChoices: () => CHOICES } satisfies WidgetRenderer;
