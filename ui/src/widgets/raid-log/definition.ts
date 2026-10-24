import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { RAIDS_AVAILABILITY } from '../../dashboard/raids.ts';
import { LOG_OPTIONS } from '../../charts/log/logOptions.ts';

export const OPTIONS: readonly OptionField[] = [
  { key: 'types', section: 'options', label: 'Raid types', hint: 'Leave empty to show all types.',
    kind: 'content-set', automatic: 'Every type', default: '' },
  ...LOG_OPTIONS,
];
export const definition = {
  kind: 'raid-log', version: 1, title: 'Raid log', icon: 'target',
  description: 'Raids, targets, preparation, phases and outcomes.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 7, h: 14 }, countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  availability: RAIDS_AVAILABILITY, options: OPTIONS,
} as const satisfies WidgetMeta;
