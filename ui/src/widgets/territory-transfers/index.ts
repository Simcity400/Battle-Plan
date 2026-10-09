import type { WidgetRenderer } from '../../dashboard/definitions';
import TerritoryTransfersWidget from './TerritoryTransfersWidget';
import { useConferenceChoices } from './useConferenceChoices';

export default { render: TerritoryTransfersWidget, useOptionChoices: useConferenceChoices } satisfies WidgetRenderer;
