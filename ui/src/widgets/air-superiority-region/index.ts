import type { WidgetRenderer } from '../../dashboard/definitions';
import { useAirChoices } from '../../dashboard/useAirSuperiority';
import AirSuperiorityRegionWidget from './AirSuperiorityRegionWidget';
import { OPTIONS } from './definition';
export default { render: AirSuperiorityRegionWidget, useOptionChoices: (widget, runtime) => useAirChoices(widget, runtime, OPTIONS) } satisfies WidgetRenderer;
