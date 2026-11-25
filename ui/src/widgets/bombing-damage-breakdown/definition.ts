import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { BREAKDOWN_OPTIONS } from '../../families/breakdown/options.ts';

export const OPTIONS: readonly OptionField[] = [
  { key: 'direction', section: 'options', label: 'Damage', kind: 'choice', default: 'taken',
    choices: [{ value: 'taken', label: 'Suffered' }, { value: 'dealt', label: 'Inflicted' }] },
  ...BREAKDOWN_OPTIONS,
];

export const definition = {
  kind: 'bombing-damage-breakdown', version: 1, title: 'Bombing damage breakdown', icon: 'sunburst',
  description: 'One country’s applied bombing damage, by building type and state.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 11 }, countries: { max: 1 },
  viewSwitch: { option: 'view' },
  options: OPTIONS,
} as const satisfies WidgetMeta;
