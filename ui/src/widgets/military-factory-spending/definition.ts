import type { WidgetMeta } from '../../dashboard/definitions';
import { scopedOptionKey } from '../../dashboard/options.ts';
import type { OptionField, OptionValue } from '../../dashboard/options';
import { SUNBURST_OPTIONS, levelOption } from '../../charts/sunburst/sunburstOptions.ts';
import { IC_DECIMALS_OPTION } from '../../dashboard/icDecimals.ts';
import { VIEWS } from '../dockyard-spending/definition.ts';

export const EQUIPMENT = [{ value: 'air', label: 'Air' }, { value: 'land', label: 'Land' }] as const;

const circle = (options: Readonly<Record<string, OptionValue>>) => options.view !== 'months';

export const OPTIONS: readonly OptionField[] = [
  { key: 'equipment', section: 'options', label: 'Equipment', kind: 'choice', default: 'air', choices: EQUIPMENT },
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'circle', choices: VIEWS },
  { ...levelOption('purposeLevel', 'Spent on', 'New and converted equipment'), visibleWhen: circle },
  { ...levelOption('typeLevel', 'Equipment type'), visibleWhen: circle },
  { ...levelOption('variantLevel', 'Design'), visibleWhen: circle },
  IC_DECIMALS_OPTION,
  ...SUNBURST_OPTIONS.map(field => ({ ...field, visibleWhen: circle })),
];

const variants = EQUIPMENT.flatMap(equipment => VIEWS.map(view => ({ equipment, view })))
  .filter(({ equipment, view }) => equipment.value !== 'air' || view.value !== 'circle')
  .map(({ equipment, view }) => ({
    key: `equipment=${equipment.value},view=${view.value}`,
    initial: { title: `${equipment.label} factory spending${view.value === 'months' ? ' by month' : ''}`,
      options: {
        ...equipment.value === 'air' ? {} : { [view.value === 'months' ? scopedOptionKey('equipment', 'months') : 'equipment']: equipment.value },
        ...view.value === 'circle' ? {} : { view: view.value },
      } },
  }));

export const definition = {
  kind: 'military-factory-spending', version: 1, title: 'Military factory spending', icon: 'sunburst',
  description: 'Military factory IC spent on new and converted air or land equipment, by equipment type and design.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 11 },
  options: OPTIONS,
  viewSwitch: { option: 'view' },
  variants,
} as const satisfies WidgetMeta;
