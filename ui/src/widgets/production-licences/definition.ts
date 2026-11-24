import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'between', section: 'options', label: 'With countries chosen, show', kind: 'choice', default: 'touching',
    choices: [{ value: 'touching', label: 'Their licences with anyone' }, { value: 'between', label: 'Only licences between them' }] },
];

export const definition = {
  kind: 'production-licences', version: 2, title: 'Production licences', icon: 'factory', category: 'Economy',
  description: 'Who builds whose designs under licence.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 10 }, countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  defaultCountries: { kind: 'all' }, options: OPTIONS,
} as const satisfies WidgetMeta;
