import type { WidgetRenderer } from '../../dashboard/definitions';
import { useAirChoices } from '../../dashboard/useAirSuperiority';
import AirSuperiorityTimelineWidget from './AirSuperiorityTimelineWidget';
export default { render: AirSuperiorityTimelineWidget, useOptionChoices: useAirChoices } satisfies WidgetRenderer;
