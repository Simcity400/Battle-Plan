import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { LOG_OPTIONS } from '../../charts/log/logOptions.ts';

export const CATEGORIES = [
  { value: 'warsPeace', label: 'Wars & peace', palette: 1 },
  { value: 'diplomacy', label: 'Diplomacy', palette: 0 },
  { value: 'politics', label: 'Politics', palette: 5 },
  { value: 'milestones', label: 'Milestones', palette: 2 },
] as const;

export const OPTIONS: readonly OptionField[] = [
  ...CATEGORIES.map(category => ({ key: category.value, section: 'options' as const, label: category.label,
    kind: 'toggle' as const, default: true })),
  { key: 'artwork', section: 'options', label: 'Game artwork', kind: 'toggle', default: true },
  ...LOG_OPTIONS,
];

export const definition = {
  kind: 'event-feed', version: 1, title: 'Event feed', icon: 'clock',
  description: 'Key moments across every country, dated by the recording that establishes them.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 7, h: 14 },
  countries: { max: Number.MAX_SAFE_INTEGER, all: true }, options: OPTIONS,
} as const satisfies WidgetMeta;
