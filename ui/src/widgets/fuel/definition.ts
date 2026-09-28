import type { WidgetMeta } from '../../dashboard/definitions';
import { AFTER_PLAYHEAD_OPTION } from '../../dashboard/options.ts';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'playback', section: 'time', label: 'While following playback', kind: 'choice', default: 'draw-in',
    choices: [{ value: 'draw-in', label: 'Grow to now' }, { value: 'cursor', label: 'Whole campaign' }, { value: 'window', label: 'Recent window' }] },
  AFTER_PLAYHEAD_OPTION,
  { key: 'windowDays', section: 'time', label: 'Window length', kind: 'number', min: 7, max: 3650, step: 1, unit: 'days', default: 365,
    visibleWhen: options => options.playback === 'window' },
  { key: 'strip', section: 'options', label: 'Gain and burn bars', hint: 'Each day’s fuel gained and burned, under the chart', kind: 'toggle', default: true },
  { key: 'capacity', section: 'options', label: 'Capacity', hint: 'The tank’s capacity, dashed', kind: 'toggle', default: true },
  { key: 'areaFill', section: 'options', label: 'Area fill', hint: 'Shade under the fuel line', kind: 'toggle', default: true },
  { key: 'endLabels', section: 'options', label: 'Values at line ends', kind: 'toggle', default: true },
  { key: 'yMin', section: 'axes', label: 'Value axis from', hint: 'Empty starts at zero', kind: 'number', min: -1e9, max: 1e9, step: 1, default: null },
  { key: 'yMax', section: 'axes', label: 'Value axis to', hint: 'Empty fits the data', kind: 'number', min: -1e9, max: 1e9, step: 1, default: null },
  { key: 'log', section: 'axes', label: 'Log scale', hint: 'Only when every value shown is positive', kind: 'toggle', default: false },
];

export const definition = {
  kind: 'fuel', version: 1, title: 'Fuel', icon: 'droplet',
  description: 'A country’s fuel against its capacity over time, with each day’s gain and burn.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 12, h: 10 },
  options: OPTIONS,
} as const satisfies WidgetMeta;
