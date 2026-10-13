import type { WidgetMeta } from '../../dashboard/definitions';
import { REFITS_OPTION } from '../../dashboard/equipment.ts';
import type { OptionField } from '../../dashboard/options';
import { SUNBURST_OPTIONS, levelOption } from '../../charts/sunburst/sunburstOptions.ts';
import { IC_DECIMALS_OPTION } from '../../dashboard/icDecimals.ts';

export const OPTIONS: readonly OptionField[] = [
  levelOption('domainLevel', 'Production source', 'Land, air, naval and Rocket sites'),
  levelOption('typeLevel', 'Equipment type'),
  levelOption('variantLevel', 'Design'),
  REFITS_OPTION,
  IC_DECIMALS_OPTION,
  ...SUNBURST_OPTIONS,
];

export const definition = {
  kind: 'production-spending', version: 1, title: 'Production spending', icon: 'sunburst',
  description: 'Factory IC spent by source, equipment type and design.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 11 },
  options: OPTIONS,
} as const satisfies WidgetMeta;
