import type { WidgetMeta } from '../../dashboard/definitions';
import { REFITS_OPTION } from '../../dashboard/equipment.ts';
import type { OptionField } from '../../dashboard/options';
import { SUNBURST_OPTIONS, levelOption } from '../../charts/sunburst/sunburstOptions.ts';

export const OPTIONS: readonly OptionField[] = [
  levelOption('domainLevel', 'Land, air and naval', 'The innermost ring groups equipment by the engine’s arms'),
  levelOption('typeLevel', 'Equipment type', 'A ring of equipment types from the campaign’s content'),
  levelOption('variantLevel', 'Exact design', 'A ring of recorded equipment designs'),
  REFITS_OPTION,
  ...SUNBURST_OPTIONS,
];

export const definition = {
  kind: 'production-spending', version: 1, title: 'Production spending', icon: 'sunburst',
  description: 'Production-line deliveries weighted by the cost at their read, by arm, equipment type and exact design.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 11 },
  options: OPTIONS,
} as const satisfies WidgetMeta;
