import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { SUNBURST_OPTIONS, levelOption } from '../../charts/sunburst/sunburstOptions.ts';
import { IC_DECIMALS_OPTION } from '../../dashboard/icDecimals.ts';

export const OPTIONS: readonly OptionField[] = [
  levelOption('splitLevel', 'New or repair'),
  levelOption('typeLevel', 'Building type'),
  levelOption('stateLevel', 'State'),
  IC_DECIMALS_OPTION,
  ...SUNBURST_OPTIONS,
];

export const definition = {
  kind: 'construction-spending', version: 1, title: 'Construction spending', icon: 'sunburst',
  description: 'Where a country’s civilian-factory output went: new buildings and repairs, each building type and each state.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 11 },
  options: OPTIONS,
} as const satisfies WidgetMeta;
