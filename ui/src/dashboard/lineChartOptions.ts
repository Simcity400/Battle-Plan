import { AFTER_PLAYHEAD_OPTION } from './options.ts';
import type { OptionField } from './options';

export const TIME_CHART_TIME_OPTIONS: readonly OptionField[] = [
  { key: 'playback', section: 'time', label: 'While following playback', kind: 'choice', default: 'draw-in',
    choices: [{ value: 'draw-in', label: 'Grow to now' }, { value: 'window', label: 'Recent window' }],
    visibleWhen: (_options, _countries, time) => time === undefined || time === 'follow' },
  { key: 'windowDays', section: 'time', label: 'Window length', kind: 'number', min: 7, max: 3650, step: 1, unit: 'days', default: 365,
    visibleWhen: (options, _countries, time) => options.playback === 'window' && (time === undefined || time === 'follow') },
];
export const LINES_VIEW_OPTIONS: readonly OptionField[] = [...TIME_CHART_TIME_OPTIONS, { ...AFTER_PLAYHEAD_OPTION, visibleWhen: () => false }];
export const END_LABELS_OPTION: OptionField = { key: 'endLabels', section: 'options', label: 'Names at line ends', kind: 'toggle', default: true };
export const LINE_AXIS_OPTIONS: readonly OptionField[] = [
  { key: 'yMin', section: 'axes', label: 'Value axis from', hint: 'Empty fits the data', kind: 'number', min: -1e9, max: 1e9, step: 1, default: null },
  { key: 'yMax', section: 'axes', label: 'Value axis to', hint: 'Empty fits the data', kind: 'number', min: -1e9, max: 1e9, step: 1, default: null },
  { key: 'log', section: 'axes', label: 'Log scale', hint: 'Only when every value shown is positive', kind: 'toggle', default: false },
];
