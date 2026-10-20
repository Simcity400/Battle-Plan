import type { WidgetRenderer } from '../../dashboard/definitions';
import { useFactionChoices } from '../../dashboard/factionChoice';
import FactionPooledStrengthWidget from './FactionPooledStrengthWidget';

export default { render: FactionPooledStrengthWidget, useOptionChoices: useFactionChoices } satisfies WidgetRenderer;
