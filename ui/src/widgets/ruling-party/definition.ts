import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'changes', section: 'options', label: 'Government changes and elections', kind: 'toggle', default: true },
  { key: 'leaders', section: 'options', label: 'Leader changes', kind: 'toggle', default: true },
];

export const definition = {
  kind: 'ruling-party',
  version: 1,
  title: 'Ruling party',
  icon: 'palette',
  description: 'The country leader and party popularity over time, with changes of government, leader and elections.',
  timeModes: ['campaign', 'follow', 'pin', 'range'],
  defaultTime: 'follow',
  scope: 'history',
  data: true,
  history: true,
  minSize: { w: 4, h: 6 },
  defaultSize: { w: 8, h: 10 },
  countries: { max: 1 },
  options: OPTIONS,
} as const satisfies WidgetMeta;
