import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'sort', section: 'options', label: 'Sort', kind: 'choice', default: 'gained',
    choices: [{ value: 'gained', label: 'Date gained' }, { value: 'ended', label: 'Date ended' }, { value: 'name', label: 'Name' }] },
  { key: 'ended', section: 'options', label: 'Ended spirits', kind: 'choice', default: 'shown',
    choices: [{ value: 'shown', label: 'Shown' }, { value: 'hidden', label: 'Hidden' }] },
  { key: 'iconsOnBars', section: 'options', label: 'Icons on bars', kind: 'toggle', default: true },
];

export const definition = {
  kind: 'spirit-history', version: 1, title: 'National spirits history', icon: 'history',
  description: 'Every national spirit the country had, and when.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  playsBackDated: true,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 8, h: 9 },
  options: OPTIONS,
} as const satisfies WidgetMeta;
