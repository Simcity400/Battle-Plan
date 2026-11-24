import type { WidgetMeta } from '../../dashboard/definitions';
import { REFITS_OPTION } from '../../dashboard/equipment.ts';
import type { OptionField } from '../../dashboard/options';
import { SUNBURST_OPTIONS, levelOption } from '../../charts/sunburst/sunburstOptions.ts';
import { IC_DECIMALS_OPTION } from '../../dashboard/icDecimals.ts';
import { choiceVariants } from '../../dashboard/options.ts';

export const FACTORY_KINDS = [{ value: 'military', label: 'Military factories' }, { value: 'dockyards', label: 'Dockyards' }] as const;
export type FactoryKind = typeof FACTORY_KINDS[number]['value'];

export const OPTIONS: readonly OptionField[] = [
  { key: 'factories', section: 'options', label: 'Factories', kind: 'choice', default: 'military', choices: FACTORY_KINDS },
  levelOption('domainLevel', 'Production source', 'Land, air and rocket sites; or naval'),
  levelOption('typeLevel', 'Equipment type'),
  levelOption('variantLevel', 'Design'),
  REFITS_OPTION,
  IC_DECIMALS_OPTION,
  ...SUNBURST_OPTIONS,
];

export const definition = {
  kind: 'production-spending', version: 1, title: 'Military production spending', icon: 'sunburst',
  description: 'Military factory or dockyard IC spent by source, equipment type and design.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 11 },
  options: OPTIONS,
  variants: choiceVariants(OPTIONS.find(field => field.key === 'factories'), () => 'Naval production spending'),
} as const satisfies WidgetMeta;
