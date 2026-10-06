import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { RAIDS_AVAILABILITY } from '../../dashboard/raids.ts';
import { LOG_OPTIONS } from '../../charts/log/logOptions.ts';

export const OPTIONS: readonly OptionField[] = [
  { key: 'types', section: 'options', label: 'Raid types', hint: 'The recording’s content types; none chosen shows every type',
    kind: 'content-set', automatic: 'Every type', max: 16, default: '' },
  ...LOG_OPTIONS,
];
export const definition = {
  kind: 'raid-log', version: 1, title: 'Raid log', icon: 'target',
  description: 'Every country’s raids: target, preparation, phase and outcome, dated by their recorded reads.',
  timeModes: ['campaign', 'follow', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 7, h: 14 }, countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  availability: RAIDS_AVAILABILITY, options: OPTIONS,
} as const satisfies WidgetMeta;
