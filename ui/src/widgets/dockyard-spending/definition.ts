import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField, OptionValue } from '../../dashboard/options';
import { IC_DECIMALS_OPTION } from '../../dashboard/icDecimals.ts';
import { BREAKDOWN_VIEWS } from '../../families/breakdown/options.ts';

export const VIEWS = [...BREAKDOWN_VIEWS, { value: 'months', label: 'By month' }] as const;

const ring = (options: Readonly<Record<string, OptionValue>>) => options.view === 'ring';

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'parts', choices: VIEWS },
  { key: 'namesInRing', section: 'options', label: 'Names in ring', kind: 'toggle', default: true, visibleWhen: ring },
  IC_DECIMALS_OPTION,
];

export const definition = {
  kind: 'dockyard-spending', version: 1, title: 'Dockyard spending', icon: 'sunburst',
  description: 'Dockyard IC spent on new ships, refits and repairs, by equipment type and design.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 11 },
  options: OPTIONS,
  viewSwitch: { option: 'view' },
  variants: [{ key: 'view=months', initial: { title: 'Dockyard spending by month', options: { view: 'months' } } }],
} as const satisfies WidgetMeta;
