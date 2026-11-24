import type { WidgetMeta } from '../../dashboard/definitions';
import { AFTER_PLAYHEAD_OPTION } from '../../dashboard/options.ts';
import type { OptionField } from '../../dashboard/options';
import { breakdownOption, DRILL_OPTION, breakdownVariants } from '../../charts/drill/drillState.ts';
import type { Breakdown } from '../../charts/drill/drillState';

export const STABILITY_BREAKDOWNS: readonly Breakdown[] = [
  { id: 'stability', label: 'Stability parts' },
  { id: 'warSupport', label: 'War support parts' },
];

export const STABILITY_OPTIONS: readonly OptionField[] = [
  { key: 'playback', section: 'time', label: 'While following playback', kind: 'choice', default: 'draw-in',
    choices: [{ value: 'draw-in', label: 'Grow to now' }, { value: 'cursor', label: 'Whole campaign' }, { value: 'window', label: 'Recent window' }] },
  AFTER_PLAYHEAD_OPTION,
  { key: 'windowDays', section: 'time', label: 'Window length', kind: 'number', min: 7, max: 3650, step: 1, unit: 'days', default: 365,
    visibleWhen: options => options.playback === 'window' },
  { key: 'stability', section: 'options', label: 'Stability', kind: 'toggle', default: true },
  { key: 'warSupport', section: 'options', label: 'War support', kind: 'toggle', default: true },
  { key: 'endLabels', section: 'options', label: 'Values at line ends', kind: 'toggle', default: true },
  { key: 'markWars', section: 'options', label: 'Markers: wars and capitulations', kind: 'toggle', default: true },
  { key: 'markFactions', section: 'options', label: 'Markers: faction changes', kind: 'toggle', default: true },
  { key: 'markGovernment', section: 'options', label: 'Markers: government changes', hint: 'Laws, advisors and ruling party', kind: 'toggle', default: true },
  { key: 'markFocus', section: 'options', label: 'Markers: focuses completed', kind: 'toggle', default: true },
  { key: 'scale', section: 'axes', label: 'Scale', kind: 'choice', default: 'fixed',
    choices: [{ value: 'fixed', label: '0–100%' }, { value: 'zoom', label: 'Zoom' }] },
  breakdownOption(STABILITY_BREAKDOWNS, 'Stability and war support'),
  DRILL_OPTION,
];

export const definition = {
  kind: 'stability', version: 1, title: 'Stability & War support', icon: 'chart-line',
  description: 'Stability and war support over time, with what makes up each value.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 12, h: 10 },
  countries: { max: Number.MAX_SAFE_INTEGER },
  options: STABILITY_OPTIONS,
  variants: breakdownVariants('Stability & War support', STABILITY_OPTIONS),
} as const satisfies WidgetMeta;
