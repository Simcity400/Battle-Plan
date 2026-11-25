import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { BREAKDOWN_OPTIONS } from '../../families/breakdown/options.ts';

export const OPTIONS: readonly OptionField[] = [
  { key: 'landSource', section: 'options', label: 'Land losses from', kind: 'choice', default: 'destruction',
    choices: [{ value: 'destruction', label: 'Every destruction' }, { value: 'combatLog', label: "Game's Combat Log" }] },
  ...BREAKDOWN_OPTIONS,
];

export const definition = {
  kind: 'equipment-losses', version: 1, title: 'Equipment losses', icon: 'sunburst',
  description: 'Equipment losses by cause, type and design.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 11 }, countries: { max: 1 },
  viewSwitch: { option: 'view' },
  options: OPTIONS,
} as const satisfies WidgetMeta;
