import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { partsRaceOptions } from '../../charts/race/partsRaceOptions.ts';
import { breakdownOption, breakdownVariants } from '../../charts/drill/drillState.ts';

export const BREAKDOWN = { id: 'breakdown', label: 'Holder breakdown' };
export const OPTIONS: readonly OptionField[] = partsRaceOptions('source')
  .filter(field => !['split', 'endLabels'].includes(field.key))
  .flatMap((field): OptionField[] => field.key === 'view' ? [{ ...field, default: 'race', visibleWhen: () => false } as OptionField, breakdownOption([BREAKDOWN], 'Holders')]
    : field.key === 'dateOverlay' ? [{ ...field, visibleWhen: options => options.show === 'main' }]
    : field.section === 'time' || field.section === 'axes'
    ? [{ ...field, visibleWhen: (options, countries, time) => options.show === 'breakdown' && (field.visibleWhen?.({ ...options, view: 'mix' }, countries, time) ?? true) }]
    : [field]);
export const definition = {
  kind: 'occupation-reserve-manpower', version: 1, title: 'Occupation reserve manpower', icon: 'flag', category: 'Occupation',
  description: 'Foreign garrison manpower each country holds in reserve, by the country that sent it, with its history.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER, ranked: true, all: true },
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 10 }, options: OPTIONS,
  variants: breakdownVariants('Occupation reserve manpower', OPTIONS),
} as const satisfies WidgetMeta;
