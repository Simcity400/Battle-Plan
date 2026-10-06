import type { WidgetMeta } from '../../dashboard/definitions';
import { AFTER_PLAYHEAD_OPTION } from '../../dashboard/options.ts';
import type { OptionField, OptionValue } from '../../dashboard/options';

const overTime = (options: Readonly<Record<string, OptionValue>>) => options.view === 'time';
const grouping = (options: Readonly<Record<string, OptionValue>>) => options.grouping ?? 'type';
const bars = (options: Readonly<Record<string, OptionValue>>) => options.view !== 'time';

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'bars',
    choices: [{ value: 'bars', label: 'Bars' }, { value: 'time', label: 'Over time' }] },
  { key: 'grouping', section: 'options', label: 'Group by', kind: 'choice', default: 'type',
    choices: [{ value: 'type', label: 'Equipment type' }, { value: 'variant', label: 'Exact variant' }, { value: 'domain', label: 'Land, air and naval' }] },
  { key: 'included', section: 'options', label: 'Included types', hint: 'From the campaign’s own content', kind: 'content-set',
    automatic: 'Automatic: every type recorded', max: Number.MAX_SAFE_INTEGER, default: '' },
  // One choice per grouping (`CHOSEN_KEYS`), so switching grouping keeps each one's own.
  { key: 'chosen', section: 'options', label: 'Types shown over time', hint: 'Also chosen with the chips above the chart', kind: 'content-set',
    automatic: 'Automatic: the main land and air types', max: Number.MAX_SAFE_INTEGER, default: '', visibleWhen: options => overTime(options) && grouping(options) === 'type' },
  { key: 'chosenVariant', section: 'options', label: 'Variants shown over time', hint: 'Also chosen with the chips above the chart', kind: 'content-set',
    automatic: 'Automatic: the five with the most in stock', max: Number.MAX_SAFE_INTEGER, default: '', visibleWhen: options => overTime(options) && grouping(options) === 'variant' },
  { key: 'chosenDomain', section: 'options', label: 'Arms shown over time', hint: 'Also chosen with the chips above the chart', kind: 'content-set',
    automatic: 'Automatic: every arm', max: Number.MAX_SAFE_INTEGER, default: '', visibleWhen: options => overTime(options) && grouping(options) === 'domain' },
  { key: 'endLabels', section: 'options', label: 'Names at the line ends', kind: 'toggle', default: true, visibleWhen: overTime },
  { key: 'order', section: 'options', label: 'Order', kind: 'choice', default: 'game', visibleWhen: bars,
    choices: [{ value: 'game', label: 'Game order' }, { value: 'shortage', label: 'Biggest shortage first' }, { value: 'stock', label: 'Most in stock first' }] },
  { key: 'scale', section: 'options', label: 'Bar length', kind: 'choice', default: 'campaign', visibleWhen: bars,
    choices: [{ value: 'campaign', label: 'Same scale all campaign' }, { value: 'moment', label: 'Fit the moment shown' }] },
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
  kind: 'equipment-stockpile', version: 1, title: 'Equipment stockpile', icon: 'grid',
  description: 'Each equipment type’s surplus or shortage as a share of what is needed, and what a country holds and makes per week.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 13 },
  options: OPTIONS,
} as const satisfies WidgetMeta;
