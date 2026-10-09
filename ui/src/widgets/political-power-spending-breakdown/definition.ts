import type { WidgetMeta } from '../../dashboard/definitions';
import { SUNBURST_OPTIONS, levelOption } from '../../charts/sunburst/sunburstOptions.ts';

export const OPTIONS = [
  levelOption('categoryLevel', 'Category', 'The innermost ring groups the recorded cause categories'),
  levelOption('itemLevel', 'Specific item', 'A ring of the content items or recorded targets charged'),
  ...SUNBURST_OPTIONS,
];
export const definition = {
  kind: 'political-power-spending-breakdown', version: 1, title: 'Political power spending breakdown', icon: 'sunburst',
  description: 'Where a country’s recorded political power went, grouped by category and specific item.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 11 }, countries: { max: Number.MAX_SAFE_INTEGER }, options: OPTIONS,
} as const satisfies WidgetMeta;
