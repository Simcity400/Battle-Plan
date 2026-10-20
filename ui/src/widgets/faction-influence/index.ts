import type { WidgetRenderer } from '../../dashboard/definitions';
import { useFactionChoices } from '../../dashboard/factionChoice';
import FactionInfluenceWidget from './FactionInfluenceWidget';

export default { render: FactionInfluenceWidget, useOptionChoices: useFactionChoices } satisfies WidgetRenderer;
