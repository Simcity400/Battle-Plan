import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { IC_DECIMALS_OPTION } from '../../dashboard/icDecimals.ts';
import { BREAKDOWN_OPTIONS } from '../../families/breakdown/options.ts';

export const OPTIONS: readonly OptionField[] = [...BREAKDOWN_OPTIONS, IC_DECIMALS_OPTION];

export const definition = {
  kind: 'construction-spending', version: 1, title: 'Construction spending', icon: 'sunburst',
  description: 'Where a country’s civilian-factory output went: new buildings and repairs, and each building type.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 11 }, countries: { max: 1 },
  options: OPTIONS, viewSwitch: { option: 'view' },
} as const satisfies WidgetMeta;
