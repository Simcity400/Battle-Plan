import type { WidgetMeta } from '../../dashboard/definitions';
import { AFTER_PLAYHEAD_OPTION, contentSet, choiceVariants } from '../../dashboard/options.ts';
import type { OptionField, OptionValue } from '../../dashboard/options';
import { breakdownOption, DRILL_OPTION } from '../../charts/drill/drillState.ts';

export const BREAKDOWN = { id: 'resource', label: 'Resource breakdown' };

const overTime = (options: Readonly<Record<string, OptionValue>>) => options.view === 'time';

export const OPTIONS: readonly OptionField[] = [
  breakdownOption([BREAKDOWN], 'Resources'), DRILL_OPTION,
  { key: 'resourceCountry', section: 'options', label: 'Breakdown country', kind: 'content-choice', default: '',
    automatic: 'Choose a country and date', visibleWhen: () => false },
  { key: 'partResource', section: 'options', label: 'Breakdown resource', kind: 'content-choice', default: '',
    automatic: 'First resource',
    visibleWhen: options => options.show === 'resource' },
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'bars',
    choices: [{ value: 'bars', label: 'Bars' }, { value: 'time', label: 'Time' }] },
  { key: 'resources', section: 'options', label: 'Resources over time', kind: 'content-set',
    automatic: 'Largest resource', default: '', visibleWhen: overTime },
  { key: 'gap', section: 'options', label: 'Shade surplus and shortfall', kind: 'toggle', default: true,
    visibleWhen: options => options.show !== 'resource' && overTime(options) && contentSet(options.resources ?? '').length <= 1 },
  { key: 'hideEmpty', section: 'options', label: 'Hide unused resources', kind: 'toggle', default: false,
    visibleWhen: options => options.view === 'bars' },
  { key: 'playback', section: 'time', label: 'While following playback', kind: 'choice', default: 'draw-in', visibleWhen: overTime,
    choices: [{ value: 'draw-in', label: 'Grow to now' }, { value: 'cursor', label: 'Whole campaign' }, { value: 'window', label: 'Recent window' }] },
  { ...AFTER_PLAYHEAD_OPTION, visibleWhen: options => overTime(options) && options.playback === 'cursor' },
  { key: 'windowDays', section: 'time', label: 'Window length', kind: 'number', min: 7, max: 3650, step: 1, unit: 'days', default: 365,
    visibleWhen: options => overTime(options) && options.playback === 'window' },
  { key: 'yMin', section: 'axes', label: 'Value axis from', hint: 'Empty fits the data', kind: 'number', min: -1e9, max: 1e9, step: 1, default: null, visibleWhen: overTime },
  { key: 'yMax', section: 'axes', label: 'Value axis to', hint: 'Empty fits the data', kind: 'number', min: -1e9, max: 1e9, step: 1, default: null, visibleWhen: overTime },
  { key: 'log', section: 'axes', label: 'Log scale', hint: 'Only when every value shown is positive', kind: 'toggle', default: false, visibleWhen: overTime },
];

export const definition = {
  kind: 'resource-balance', version: 1, title: 'Resource balance', icon: 'layers',
  description: 'Where a country’s resources come from and where they go, per day, with its trade partners.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 13 },
  countries: { max: Number.MAX_SAFE_INTEGER }, options: OPTIONS,
  variants: choiceVariants(OPTIONS.find(field => field.key === 'view'), () => 'Resources over time'),
} as const satisfies WidgetMeta;
