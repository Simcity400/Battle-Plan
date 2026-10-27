import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { partsRaceOptions } from '../../charts/race/partsRaceOptions.ts';
import { breakdownVariants } from '../../charts/drill/drillState.ts';

export const BREAKDOWN = { id: 'breakdown', label: 'Holder breakdown' };
/** The parts race's settings, as a race only: one bar per holder split by source country. */
export const OPTIONS: readonly OptionField[] = partsRaceOptions('source', BREAKDOWN.label)
  .filter(field => !['split', 'endLabels'].includes(field.key))
  .map(field => field.key === 'view' ? { ...field, default: 'race', visibleWhen: () => false } as OptionField
    : field.key === 'show' && field.kind === 'choice'
    ? { ...field, choices: field.choices.map(choice => choice.value === 'main' ? { ...choice, label: 'Holders' } : choice) } : field);
export const definition = {
  kind: 'occupation-reserve-manpower', version: 1, title: 'Occupation reserve manpower', icon: 'flag', category: 'Occupation',
  description: 'Foreign garrison manpower each country holds in reserve, by the country that sent it, with its history.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER, ranked: true, all: true },
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 10 }, options: OPTIONS,
  variants: breakdownVariants('Occupation reserve manpower', OPTIONS),
} as const satisfies WidgetMeta;
