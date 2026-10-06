import type { WidgetMeta } from '../../dashboard/definitions';
import { AFTER_PLAYHEAD_OPTION } from '../../dashboard/options.ts';
import type { OptionField, OptionValue } from '../../dashboard/options';

const overTime = (options: Readonly<Record<string, OptionValue>>) => options.view === 'time';

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'bars',
    choices: [{ value: 'bars', label: 'Balance bars' }, { value: 'time', label: 'Over time' }] },
  { key: 'resources', section: 'options', label: 'Resources over time', hint: 'From the campaign’s own content', kind: 'content-set',
    automatic: 'Automatic: the largest resource', default: '', visibleWhen: overTime },
  { key: 'gap', section: 'options', label: 'Shade surplus and shortfall', hint: 'With one resource chosen', kind: 'toggle', default: true, visibleWhen: overTime },
  { key: 'order', section: 'options', label: 'Order', kind: 'choice', default: 'game', visibleWhen: options => options.view === 'bars',
    choices: [{ value: 'game', label: 'Game order' }, { value: 'shortfall', label: 'Biggest shortfall first' }, { value: 'volume', label: 'Largest first' }] },
  { key: 'scale', section: 'options', label: 'Bar length', kind: 'choice', default: 'campaign', visibleWhen: options => options.view === 'bars',
    choices: [{ value: 'campaign', label: 'Same scale all campaign' }, { value: 'moment', label: 'Fit the moment shown' }] },
  { key: 'hideEmpty', section: 'options', label: 'Hide resources with nothing', hint: 'No supply and no demand at the moment shown', kind: 'toggle', default: false,
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
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 13 },
  options: OPTIONS,
} as const satisfies WidgetMeta;
