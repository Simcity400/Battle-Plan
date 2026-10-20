import type { WidgetRenderer } from '../../dashboard/definitions';
import { useFactionChoices } from '../../dashboard/factionChoice';
import FactionGoalsWidget from './FactionGoalsWidget';

export default { render: FactionGoalsWidget, useOptionChoices: useFactionChoices } satisfies WidgetRenderer;
