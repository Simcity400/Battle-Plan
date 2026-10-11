import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { LOG_OPTIONS } from '../../charts/log/logOptions.ts';

export const OPTIONS: readonly OptionField[] = [
  { key: 'countryRole', section: 'countries', label: 'Match countries as', hint: 'Filter by the credited country.', kind: 'choice', default: 'both',
    choices: [{ value: 'both', label: 'Victim or sinker' }, { value: 'victim', label: 'Victim' }, { value: 'sinker', label: 'Sinker' }] },
  { key: 'classes', section: 'options', label: 'Ship classes', kind: 'content-set',
    automatic: 'Every class', default: '' },
  ...LOG_OPTIONS,
];

export const definition = {
  kind: 'ships-sunk', version: 1, title: 'Ships sunk log', icon: 'history',
  description: 'Ships sunk, with their owners, credited countries, classes and sea zones.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 7, h: 14 }, countries: { max: Number.MAX_SAFE_INTEGER, all: true }, options: OPTIONS,
} as const satisfies WidgetMeta;
