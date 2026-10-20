import type { WidgetRenderer } from '../../dashboard/definitions';
import AirStatisticsWidget from './AirStatisticsWidget';
import { useAirStatisticsChoices } from './useAirStatistics';
export default { render: AirStatisticsWidget, useOptionChoices: useAirStatisticsChoices } satisfies WidgetRenderer;
