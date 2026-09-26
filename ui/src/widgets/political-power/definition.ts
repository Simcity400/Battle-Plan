import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'playback', section: 'time', label: 'While following playback', kind: 'choice', default: 'draw-in',
    choices: [{ value: 'draw-in', label: 'Grow to now' }, { value: 'cursor', label: 'Whole campaign' }, { value: 'window', label: 'Recent window' }] },
  { key: 'windowDays', section: 'time', label: 'Window length', kind: 'number', min: 7, max: 3650, step: 1, unit: 'days', default: 365,
    visibleWhen: options => options.playback === 'window' },
  { key: 'spendTicks', section: 'options', label: 'Spend ticks', hint: 'Mark each hour the balance fell', kind: 'toggle', default: true },
  { key: 'areaFill', section: 'options', label: 'Area fill', hint: "Shade under a single country's line", kind: 'toggle', default: true },
  { key: 'endLabels', section: 'options', label: 'Names at line ends', kind: 'toggle', default: true },
  { key: 'yMin', section: 'axes', label: 'Value axis from', hint: 'Empty fits the data', kind: 'number', min: -1e9, max: 1e9, step: 1, default: null },
  { key: 'yMax', section: 'axes', label: 'Value axis to', hint: 'Empty fits the data', kind: 'number', min: -1e9, max: 1e9, step: 1, default: null },
  { key: 'log', section: 'axes', label: 'Log scale', hint: 'Only when every value shown is positive', kind: 'toggle', default: false },
];

export const definition = {
  kind: 'political-power', version: 1, title: 'Political power', icon: 'sparkle',
  description: 'Each country’s political power balance over time, with the days it was spent.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 10 },
  countries: { max: 16 },
  options: OPTIONS,
} as const satisfies WidgetMeta;
