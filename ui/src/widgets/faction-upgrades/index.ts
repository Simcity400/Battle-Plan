import type { WidgetRenderer } from '../../dashboard/definitions';
import { useFactionChoices } from '../../dashboard/factionChoice';
import FactionUpgradesWidget from './FactionUpgradesWidget';

export default { render: FactionUpgradesWidget, useOptionChoices: useFactionChoices } satisfies WidgetRenderer;
