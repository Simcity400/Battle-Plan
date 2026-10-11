import { TOP_COUNTRIES } from '../../charts/race/options.ts';
import { partsRaceOptions } from '../../charts/race/partsRaceOptions.ts';
import type { WidgetMeta } from '../../dashboard/definitions';
import { REFITS_OPTION } from '../../dashboard/equipment.ts';
import type { OptionField } from '../../dashboard/options';
import { GROUPINGS, MEASURES } from './producedData.ts';
import { breakdownOption } from '../../charts/drill/drillState.ts';

export const OPTIONS: readonly OptionField[] = [
  ...partsRaceOptions('equipment').map(field => field.key === 'show'
    ? breakdownOption([{ id: 'breakdown', label: 'Equipment produced breakdown' }], 'Equipment produced') : field),
  { key: 'measure', section: 'options', label: 'Measure', hint: 'Uses the previous week or month.',
    kind: 'choice', default: 'total', choices: MEASURES.map(item => ({ value: item.value, label: item.label })) },
  { key: 'grouping', section: 'options', label: 'Group by', kind: 'choice', default: 'type',
    choices: GROUPINGS.map(item => ({ value: item.value, label: item.label })) },
  { key: 'included', section: 'options', label: 'Included types', kind: 'content-set',
    automatic: 'All types', default: '' },
  REFITS_OPTION,
];

export const definition = {
  kind: 'equipment-produced', version: 1, title: 'Equipment produced', icon: 'layers',
  description: 'Equipment production by country and type.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 12 },
  countries: { max: Number.MAX_SAFE_INTEGER, ranked: true },
  defaultCountries: TOP_COUNTRIES,
  options: OPTIONS,
} as const satisfies WidgetMeta;
