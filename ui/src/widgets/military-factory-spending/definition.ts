import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { IC_DECIMALS_OPTION } from '../../dashboard/icDecimals.ts';
import {
  BREAKDOWN_MONTH_OPTIONS,
  BREAKDOWN_MONTH_VIEWS,
  isNotMonthView,
  levelOption,
} from '../../families/breakdown/options.ts';

export const EQUIPMENT = [{ value: 'air', label: 'Air' }, { value: 'land', label: 'Land' }] as const;
export type FactoryEquipmentChoice = typeof EQUIPMENT[number]['value'];

export const OPTIONS: readonly OptionField[] = [
  { key: 'equipment', section: 'options', label: 'Equipment', kind: 'choice', default: 'air', choices: EQUIPMENT },
  ...BREAKDOWN_MONTH_OPTIONS,
  { ...levelOption('purposeLevel', 'Spent on', 'New and converted equipment'), visibleWhen: isNotMonthView },
  { ...levelOption('typeLevel', 'Equipment type'), visibleWhen: isNotMonthView },
  { ...levelOption('variantLevel', 'Design'), visibleWhen: isNotMonthView },
  IC_DECIMALS_OPTION,
];

const variants = EQUIPMENT.flatMap(equipment => BREAKDOWN_MONTH_VIEWS.filter(v => v.value === 'parts' || v.value === 'months').map(view => ({ equipment, view })))
  .filter(({ equipment, view }) => equipment.value !== 'air' || view.value !== 'parts')
  .map(({ equipment, view }) => ({
    key: `equipment=${equipment.value},view=${view.value}`,
    initial: {
      title: `${equipment.label} factory spending${view.value === 'months' ? ' by month' : ''}`,
      options: {
        ...(equipment.value === 'air' ? {} : { equipment: equipment.value }),
        ...(view.value === 'parts' ? {} : { view: view.value }),
      },
    },
  }));

export const definition = {
  kind: 'military-factory-spending', version: 1, title: 'Military factory spending', icon: 'sunburst',
  description: 'Military factory IC spent on new and converted air or land equipment, by equipment type and design.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  playsBackDated: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 11 },
  options: OPTIONS,
  viewSwitch: { option: 'view' },
  variants,
} as const satisfies WidgetMeta;
