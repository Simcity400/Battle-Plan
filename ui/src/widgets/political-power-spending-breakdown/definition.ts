import type { WidgetMeta } from '../../dashboard/definitions';
import { POWER_DECIMALS_OPTION } from '../../dashboard/powerDecimals.ts';
import { BREAKDOWN_OPTIONS, levelOption } from '../../families/breakdown/options.ts';

export const OPTIONS = [
  ...BREAKDOWN_OPTIONS,
  levelOption('categoryLevel', 'Category'),
  levelOption('itemLevel', 'Specific item'),
  POWER_DECIMALS_OPTION,
];

export const definition = {
  kind: 'political-power-spending-breakdown', version: 1, title: 'Political power spending breakdown', icon: 'sunburst',
  description: 'Political power spending by category and item.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  playsBackDated: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 11 }, countries: { max: Number.MAX_SAFE_INTEGER },
  options: OPTIONS, viewSwitch: { option: 'view' },
} as const satisfies WidgetMeta;
