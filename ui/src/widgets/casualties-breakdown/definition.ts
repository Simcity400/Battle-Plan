import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { BREAKDOWN_OPTIONS } from '../../families/breakdown/options.ts';

export const OPTIONS: readonly OptionField[] = [...BREAKDOWN_OPTIONS];

export const definition = {
  kind: 'casualties-breakdown', version: 1, title: 'Casualties breakdown', icon: 'sunburst',
  description: 'A country’s losses by war and enemy.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 11 }, countries: { max: 1 },
  options: OPTIONS, viewSwitch: { option: 'view' },
} as const satisfies WidgetMeta;
