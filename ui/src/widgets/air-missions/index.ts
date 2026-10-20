import type { WidgetRenderer } from '../../dashboard/definitions';
import AirMissionsWidget from './AirMissionsWidget';
import { useAirMissionChoices } from './useAirMissions';
export default { render: AirMissionsWidget, useOptionChoices: useAirMissionChoices } satisfies WidgetRenderer;
