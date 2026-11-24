import type { WidgetRenderer } from '../../dashboard/definitions';
import { airStatisticsDeclaration } from './airStatisticsProjection.ts';
import { useAirStatisticsChoices } from './useAirStatistics';
export default { declaration: airStatisticsDeclaration, useOptionChoices: useAirStatisticsChoices } satisfies WidgetRenderer;
