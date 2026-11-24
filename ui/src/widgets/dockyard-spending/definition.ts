import type { WidgetMeta } from '../../dashboard/definitions';
import { choiceVariants } from '../../dashboard/options.ts';
import type { OptionField, OptionValue } from '../../dashboard/options';
import { SUNBURST_OPTIONS, levelOption } from '../../charts/sunburst/sunburstOptions.ts';
import { IC_DECIMALS_OPTION } from '../../dashboard/icDecimals.ts';

export const VIEWS = [{ value: 'circle', label: 'Breakdown' }, { value: 'months', label: 'By month' }] as const;

const circle = (options: Readonly<Record<string, OptionValue>>) => options.view !== 'months';

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'circle', choices: VIEWS },
  { ...levelOption('purposeLevel', 'Spent on', 'New ships, refits and repairs'), visibleWhen: circle },
  { ...levelOption('typeLevel', 'Equipment type'), visibleWhen: circle },
  { ...levelOption('variantLevel', 'Design'), visibleWhen: circle },
  IC_DECIMALS_OPTION,
  ...SUNBURST_OPTIONS.map(field => ({ ...field, visibleWhen: circle })),
];

export const definition = {
  kind: 'dockyard-spending', version: 1, title: 'Dockyard spending', icon: 'sunburst',
  description: 'Dockyard IC spent on new ships, refits and repairs, by equipment type and design.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 11 },
  options: OPTIONS,
  viewSwitch: { option: 'view' },
  variants: choiceVariants(OPTIONS.find(field => field.key === 'view'), () => 'Dockyard spending by month'),
} as const satisfies WidgetMeta;
