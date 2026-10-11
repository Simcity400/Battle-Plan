import type { WidgetMeta } from '../../dashboard/definitions';
import { SUNBURST_OPTIONS, levelOption } from '../../charts/sunburst/sunburstOptions.ts';

export const OPTIONS = [
  levelOption('categoryLevel', 'Category'),
  levelOption('itemLevel', 'Specific item'),
  ...SUNBURST_OPTIONS,
];
export const definition = {
  kind: 'political-power-spending-breakdown', version: 1, title: 'Political power spending breakdown', icon: 'sunburst',
  description: 'Political power spending by category and item.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 11 }, countries: { max: Number.MAX_SAFE_INTEGER }, options: OPTIONS,
} as const satisfies WidgetMeta;
