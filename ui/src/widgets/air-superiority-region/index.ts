import type { WidgetRenderer } from '../../dashboard/definitions';
import { useAirChoices } from '../../dashboard/useAirSuperiority';
import AirSuperiorityRegionWidget from './AirSuperiorityRegionWidget';
export default { render: AirSuperiorityRegionWidget, useOptionChoices: useAirChoices } satisfies WidgetRenderer;
