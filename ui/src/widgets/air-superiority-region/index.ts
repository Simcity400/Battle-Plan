import type { FamilyViewRenderer } from '../../dashboard/definitions';
import { useAirChoices } from '../../dashboard/useAirSuperiority';
import AirSuperiorityRegionWidget from './AirSuperiorityRegionWidget';
export default { render: AirSuperiorityRegionWidget, useOptionChoices: useAirChoices } satisfies FamilyViewRenderer;
