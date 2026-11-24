import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'matrix',
    choices: [{ value: 'matrix', label: 'Matrix' }, { value: 'pairs', label: 'Top pairs' }] },
];

export const definition = {
  kind: 'who-spies-on-whom', version: 2, title: 'Who spies on whom', icon: 'target',
  description: 'Operatives in the field, spy by target, with counter-intelligence at home.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER, all: true }, defaultCountries: { kind: 'all' },
  minSize: { w: 6, h: 7 }, defaultSize: { w: 12, h: 10 }, options: OPTIONS,
} as const satisfies WidgetMeta;
