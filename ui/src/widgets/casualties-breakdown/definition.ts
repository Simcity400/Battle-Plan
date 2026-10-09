import type { WidgetMeta } from '../../dashboard/definitions';
import { breakdownOption, DRILL_OPTION } from '../../charts/drill/drillState.ts';
import { SUNBURST_OPTIONS, levelOption } from '../../charts/sunburst/sunburstOptions.ts';

export const DETAIL = { id: 'details', label: 'Recorded losses' };
export const definition = {
  kind: 'casualties-breakdown', version: 1, title: 'Casualties breakdown', icon: 'sunburst',
  description: 'One country’s recorded losses by native war episode and enemy, with exact signed details.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 11 },
  options: [levelOption('warLevel', 'War'), levelOption('enemyLevel', 'Enemy country'),
    ...SUNBURST_OPTIONS, DRILL_OPTION, breakdownOption([DETAIL], 'Sunburst')],
} as const satisfies WidgetMeta;
