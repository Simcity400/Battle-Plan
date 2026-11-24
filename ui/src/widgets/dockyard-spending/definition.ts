import type { WidgetMeta } from '../../dashboard/definitions';
import { choiceVariants } from '../../dashboard/options.ts';
import type { OptionField } from '../../dashboard/options';
import { IC_DECIMALS_OPTION } from '../../dashboard/icDecimals.ts';
import {
  BREAKDOWN_MONTH_OPTIONS,
  BREAKDOWN_MONTH_VIEWS,
  isNotMonthView,
  levelOption,
  type BreakdownOrMonthView,
} from '../../families/breakdown/options.ts';

export const VIEWS = BREAKDOWN_MONTH_VIEWS;
export type DockyardViewChoice = BreakdownOrMonthView;

export const OPTIONS: readonly OptionField[] = [
  ...BREAKDOWN_MONTH_OPTIONS,
  { ...levelOption('purposeLevel', 'Spent on', 'New ships, refits and repairs'), visibleWhen: isNotMonthView },
  { ...levelOption('typeLevel', 'Equipment type'), visibleWhen: isNotMonthView },
  { ...levelOption('variantLevel', 'Design'), visibleWhen: isNotMonthView },
  IC_DECIMALS_OPTION,
];

export const definition = {
  kind: 'dockyard-spending', version: 1, title: 'Dockyard spending', icon: 'sunburst',
  description: 'Dockyard IC spent on new ships, refits and repairs, by equipment type and design.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  playsBackDated: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 11 },
  options: OPTIONS,
  viewSwitch: { option: 'view' },
  variants: choiceVariants(OPTIONS.find(field => field.key === 'view'), (choice) => choice.value === 'months' ? 'Dockyard spending by month' : `Dockyard spending (${choice.label.toLowerCase()})`),
} as const satisfies WidgetMeta;
