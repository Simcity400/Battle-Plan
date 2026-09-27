import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

/** Settings as signed off (mock-ups politics-1/2): either line can be switched
 * off, the scale is fixed 0–100% or zooms to the data, line playback is
 * selectable (draw-in by default; the whole campaign stops at the playhead
 * unless set to show past it), and values sit at the line ends. */
export const STABILITY_OPTIONS: readonly OptionField[] = [
  { key: 'playback', section: 'time', label: 'While following playback', kind: 'choice', default: 'draw-in',
    choices: [{ value: 'draw-in', label: 'Grow to now' }, { value: 'cursor', label: 'Whole campaign' }, { value: 'window', label: 'Recent window' }] },
  { key: 'afterPlayhead', section: 'time', label: 'Show after the playhead', hint: 'Draws the lines, events and hover past the moment shown',
    kind: 'toggle', default: false, visibleWhen: options => options.playback === 'cursor' },
  { key: 'windowDays', section: 'time', label: 'Window length', kind: 'number', min: 7, max: 3650, step: 1, unit: 'days', default: 365,
    visibleWhen: options => options.playback === 'window' },
  { key: 'stability', section: 'options', label: 'Stability', hint: 'The solid line', kind: 'toggle', default: true },
  { key: 'warSupport', section: 'options', label: 'War support', hint: 'The dashed line', kind: 'toggle', default: true },
  { key: 'endLabels', section: 'options', label: 'Values at line ends', kind: 'toggle', default: true },
  { key: 'markWars', section: 'options', label: 'Markers: wars and capitulations', hint: '⚔ along the top', kind: 'toggle', default: true },
  { key: 'markFactions', section: 'options', label: 'Markers: faction changes', hint: '⚑ along the top', kind: 'toggle', default: true },
  { key: 'markGovernment', section: 'options', label: 'Markers: government changes', hint: '⚙ laws, advisors and ruling party', kind: 'toggle', default: true },
  { key: 'markFocus', section: 'options', label: 'Markers: focuses completed', hint: '★ along the top', kind: 'toggle', default: true },
  { key: 'scale', section: 'axes', label: 'Scale', kind: 'choice', default: 'fixed',
    choices: [{ value: 'fixed', label: '0–100%' }, { value: 'zoom', label: 'Zoom' }] },
];

export const definition = {
  kind: 'stability', version: 1, title: 'Stability & War support', icon: 'sliders',
  description: 'Stability and war support over time, with what makes up each value.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 12, h: 10 },
  countries: { max: 16 },
  options: STABILITY_OPTIONS,
} as const satisfies WidgetMeta;
