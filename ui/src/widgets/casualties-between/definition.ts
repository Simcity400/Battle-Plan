import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'matrix',
    choices: [{ value: 'matrix', label: 'Matrix' }, { value: 'opponents', label: 'By opponent' }] },
  { key: 'between', section: 'options', label: 'With countries chosen, show', kind: 'choice', default: 'touching',
    choices: [{ value: 'touching', label: 'Their casualties with anyone' }, { value: 'between', label: 'Only casualties between them' }] },
];
export const definition = {
  kind: 'casualties-between', version: 2, title: 'Casualties between countries', icon: 'target',
  description: 'Recorded casualties dealt from each country to each enemy country.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 12 },
  countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  defaultCountries: { kind: 'all' },
  options: OPTIONS,
} as const satisfies WidgetMeta;
