import { TOP_COUNTRIES, raceViewVariants } from '../../charts/race/options.ts';
import { partsRaceOptions } from '../../charts/race/partsRaceOptions.ts';
import type { WidgetMeta } from '../../dashboard/definitions';
import { REFITS_OPTION } from '../../dashboard/equipment.ts';
import type { OptionField } from '../../dashboard/options';
import { choiceVariants } from '../../dashboard/options.ts';

const COUNTS = [
  { value: 'units', label: 'Units' },
  { value: 'ic', label: 'Factory IC' },
] as const;

const MEASURES = [
  { value: 'day', label: 'Per day' },
  { value: 'week', label: 'Per week' },
  { value: 'month', label: 'Per month' },
  { value: 'total', label: 'Since recording start' },
] as const;

const RACE_OPTIONS: readonly OptionField[] = partsRaceOptions('equipment').flatMap((field): OptionField[] => {
  if (field.key === 'split') return [];
  if (field.key === 'view' && field.kind === 'choice') return [{ ...field, choices: field.choices.filter(choice => choice.value !== 'mix') }];
  return [field];
});

export const OPTIONS: readonly OptionField[] = [
  ...RACE_OPTIONS,
  { key: 'count', section: 'options', label: 'Count', hint: 'IC is the factory IC put into equipment lines, as in Production spending: one currency, so every type adds up.',
    kind: 'choice', default: 'units', choices: COUNTS.map(item => ({ value: item.value, label: item.label })) },
  { key: 'measure', section: 'options', label: 'Measure', kind: 'choice', default: 'day',
    choices: MEASURES.map(item => ({ value: item.value, label: item.label })) },
  { key: 'included', section: 'options', label: 'Equipment', hint: 'Units count one type, the one delivered most until you choose; IC counts all equipment until you choose. Several types add up.',
    kind: 'content-set', automatic: 'Most delivered, or all equipment in IC', default: '' },
  REFITS_OPTION,
];

export const definition = {
  kind: 'equipment-produced', version: 1, title: 'Equipment produced', icon: 'layers',
  description: 'Equipment each country delivers, one type at a time, or the IC it puts into all of it.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 12 },
  countries: { max: Number.MAX_SAFE_INTEGER, ranked: true },
  defaultCountries: TOP_COUNTRIES,
  options: OPTIONS,
  viewSwitch: { option: 'count' },
  variants: [...raceViewVariants('Equipment produced', OPTIONS), ...choiceVariants(OPTIONS.find(field => field.key === 'count'), () => 'Equipment output (IC)', ['ic'])],
} as const satisfies WidgetMeta;
