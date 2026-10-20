import type { WidgetRenderer } from '../../dashboard/definitions';
import NavalSupremacyWidget from './NavalSupremacyWidget';
import { useNavalSupremacyChoices } from './useNavalSupremacy';
export default { render: NavalSupremacyWidget, useOptionChoices: useNavalSupremacyChoices } satisfies WidgetRenderer;
