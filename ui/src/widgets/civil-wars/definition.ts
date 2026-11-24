import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'ended', section: 'options', label: 'Ended civil wars', kind: 'choice', default: 'shown',
    choices: [{ value: 'shown', label: 'Shown' }, { value: 'hidden', label: 'Hidden' }] },
];

export const definition = {
  kind: 'civil-wars', version: 1, title: 'Civil wars', category: 'Politics', icon: 'flag-torn',
  description: 'Every civil war, its sides, and who won when the recording proves it.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 12, h: 10 },
  countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  options: OPTIONS,
} as const satisfies WidgetMeta;
