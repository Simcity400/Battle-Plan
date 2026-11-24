import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { choiceVariants } from '../../dashboard/options.ts';

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'meters',
    choices: [{ value: 'meters', label: 'Meters' }, { value: 'history', label: 'Over time' }] },
  { key: 'countriesAre', section: 'options', label: 'The countries chosen are', kind: 'choice', default: 'overlords',
    hint: 'Overlords show every subject they had in this campaign',
    choices: [{ value: 'overlords', label: 'Overlords' }, { value: 'subjects', label: 'Subjects' }] },
];

export const definition = {
  kind: 'autonomy', version: 1, title: 'Autonomy', icon: 'link',
  description: 'Each subject’s level and its progress towards moving up or down a level, over time.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  viewSwitch: { option: 'view' },
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 11 },
  countries: { max: Number.MAX_SAFE_INTEGER },
  options: OPTIONS,
  variants: choiceVariants(OPTIONS.find(field => field.key === 'view'), choice => `Autonomy (${choice.label})`),
} as const satisfies WidgetMeta;
