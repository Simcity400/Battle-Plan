import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { LOG_OPTIONS } from '../../charts/log/logOptions.ts';

export const OPTIONS: readonly OptionField[] = [
  { key: 'countryRole', section: 'countries', label: 'Match countries as', hint: 'Sinker matches the recorded attribution country; non-hostile causes keep their cause labels', kind: 'choice', default: 'both',
    choices: [{ value: 'both', label: 'Victim or sinker' }, { value: 'victim', label: 'Victim' }, { value: 'sinker', label: 'Sinker' }] },
  { key: 'classes', section: 'options', label: 'Ship classes', hint: 'Classes present in this recording; none chosen shows every class', kind: 'content-set',
    automatic: 'Every class', default: '' },
  ...LOG_OPTIONS,
];

export const definition = {
  kind: 'ships-sunk', version: 1, title: 'Ships sunk log', icon: 'history',
  description: 'Named ships sunk, dated by their recorded event hour, with victims, attribution, classes and sea zones.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 7, h: 14 }, countries: { max: Number.MAX_SAFE_INTEGER, all: true }, options: OPTIONS,
} as const satisfies WidgetMeta;
