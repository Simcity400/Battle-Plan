import type { WidgetMeta } from '../../dashboard/definitions';
import { scopedOptionKey } from '../../dashboard/options.ts';
import type { OptionField, OptionValue } from '../../dashboard/options';
import { IC_DECIMALS_OPTION } from '../../dashboard/icDecimals.ts';
import { VIEWS } from '../dockyard-spending/definition.ts';

export const EQUIPMENT = [{ value: 'air', label: 'Air' }, { value: 'land', label: 'Land' }] as const;

const ring = (options: Readonly<Record<string, OptionValue>>) => options.view === 'ring';

export const OPTIONS: readonly OptionField[] = [
  { key: 'equipment', section: 'options', label: 'Equipment', kind: 'choice', default: 'air', choices: EQUIPMENT },
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'parts', choices: VIEWS },
  { key: 'namesInRing', section: 'options', label: 'Names in ring', kind: 'toggle', default: true, visibleWhen: ring },
  IC_DECIMALS_OPTION,
];

const variants = [
  { key: 'equipment=land', initial: { title: 'Land factory spending', options: { [scopedOptionKey('equipment', 'parts')]: 'land' } } },
  { key: 'equipment=air,view=months', initial: { title: 'Air factory spending by month', options: { view: 'months' } } },
  { key: 'equipment=land,view=months', initial: { title: 'Land factory spending by month', options: { [scopedOptionKey('equipment', 'months')]: 'land', view: 'months' } } },
];

export const definition = {
  kind: 'military-factory-spending', version: 1, title: 'Military factory spending', icon: 'sunburst',
  description: 'Military factory IC spent on new and converted air or land equipment, by equipment type and design.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 11 },
  options: OPTIONS,
  viewSwitch: { option: 'view' },
  variants,
} as const satisfies WidgetMeta;
