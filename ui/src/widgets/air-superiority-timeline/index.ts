import type { WidgetRenderer } from '../../dashboard/definitions';
import { useAirChoices } from '../../dashboard/useAirSuperiority';
import AirSuperiorityTimelineWidget from './AirSuperiorityTimelineWidget';
import { OPTIONS } from './definition';
export default { render: AirSuperiorityTimelineWidget, useOptionChoices: (widget, runtime) => useAirChoices(widget, runtime, OPTIONS) } satisfies WidgetRenderer;
