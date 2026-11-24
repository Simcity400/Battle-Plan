import type { WidgetMeta } from '../../dashboard/definitions';
import { TIME_CHART_TIME_OPTIONS } from '../../dashboard/lineChartOptions.ts';
import type { OptionField } from '../../dashboard/options';

export const STABILITY_OPTIONS: readonly OptionField[] = [
  ...TIME_CHART_TIME_OPTIONS,
  { key: 'stability', section: 'options', label: 'Stability', kind: 'toggle', default: true },
  { key: 'warSupport', section: 'options', label: 'War support', kind: 'toggle', default: true },
  { key: 'markWars', section: 'options', label: 'Markers: wars and capitulations', kind: 'toggle', default: true },
  { key: 'markFactions', section: 'options', label: 'Markers: faction changes', kind: 'toggle', default: true },
  { key: 'markGovernment', section: 'options', label: 'Markers: government changes', hint: 'Laws, advisors and ruling party', kind: 'toggle', default: true },
  { key: 'markFocus', section: 'options', label: 'Markers: focuses completed', kind: 'toggle', default: true },
  { key: 'scale', section: 'axes', label: 'Scale', kind: 'choice', default: 'fixed',
    choices: [{ value: 'fixed', label: '0–100%' }, { value: 'zoom', label: 'Zoom' }] },
];

export const definition = {
  kind: 'stability', version: 1, title: 'Stability & War support', icon: 'chart-line',
  description: 'Stability and war support over time, with what makes up each value.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 12, h: 10 },
  countries: { max: Number.MAX_SAFE_INTEGER },
  options: STABILITY_OPTIONS,
} as const satisfies WidgetMeta;
