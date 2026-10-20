import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { NARROW_TABLE_OPTION } from '../../charts/table/options.ts';
import { DRILL_OPTION } from '../../charts/drill/drillState.ts';

export const COUNTRY = { id: 'country', label: 'Country licences' };

export const OPTIONS: readonly OptionField[] = [
  { key: 'width', section: 'options', label: 'Ribbon width', kind: 'choice', default: 'licences',
    choices: [{ value: 'licences', label: 'Licences' }, { value: 'factories', label: 'Civilian factories' }] },
  NARROW_TABLE_OPTION, DRILL_OPTION,
];

export const definition = {
  kind: 'production-licences', version: 1, title: 'Production licences', icon: 'factory', category: 'Economy',
  description: 'Who builds whose designs under licence.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 10 }, countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  defaultCountries: { kind: 'all' }, options: OPTIONS,
} as const satisfies WidgetMeta;
