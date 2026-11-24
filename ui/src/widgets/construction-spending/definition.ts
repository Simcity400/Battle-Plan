import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { IC_DECIMALS_OPTION } from '../../dashboard/icDecimals.ts';
import { BREAKDOWN_OPTIONS, levelOption } from '../../families/breakdown/options.ts';

export const OPTIONS: readonly OptionField[] = [
  ...BREAKDOWN_OPTIONS,
  levelOption('splitLevel', 'New or repair'),
  levelOption('typeLevel', 'Building type'),
  levelOption('stateLevel', 'State'),
  IC_DECIMALS_OPTION,
];

export const definition = {
  kind: 'construction-spending', version: 1, title: 'Construction spending', icon: 'sunburst',
  description: 'Where a country’s civilian-factory output went: new buildings and repairs, each building type and each state.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  playsBackDated: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 11 },
  options: OPTIONS, viewSwitch: { option: 'view' },
} as const satisfies WidgetMeta;
