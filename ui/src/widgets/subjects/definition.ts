import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'order', section: 'options', label: 'Overlords', kind: 'choice', default: 'first',
    choices: [{ value: 'first', label: 'First subject' }, { value: 'subjects', label: 'Most subjects' }, { value: 'name', label: 'Name' }] },
  { key: 'subjects', section: 'options', label: 'Subjects', kind: 'choice', default: 'since',
    choices: [{ value: 'since', label: 'Date became subject' }, { value: 'name', label: 'Name' }] },
  { key: 'ended', section: 'options', label: 'Former subjects', kind: 'choice', default: 'shown',
    choices: [{ value: 'shown', label: 'Shown' }, { value: 'hidden', label: 'Hidden' }] },
  { key: 'names', section: 'options', label: 'Level names in bars', kind: 'toggle', default: true },
];

export const definition = {
  kind: 'subjects', version: 1, title: 'Subjects', icon: 'hierarchy',
  description: 'Every overlord and its subjects over time, coloured by subject level.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 8, h: 10 },
  countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  options: OPTIONS,
} as const satisfies WidgetMeta;
