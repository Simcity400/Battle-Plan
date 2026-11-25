import type { WidgetMeta } from '../../dashboard/definitions';
import { AFTER_PLAYHEAD_OPTION, choiceVariants } from '../../dashboard/options.ts';
import type { OptionField, OptionValue } from '../../dashboard/options';
import { FUEL_DECIMALS_OPTION } from './fuelBranchesData.ts';

const overTime = (options: Readonly<Record<string, OptionValue>>) => options.view === 'time';

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'bars',
    choices: [{ value: 'bars', label: 'Bars' }, { value: 'time', label: 'Over time' }] },
  FUEL_DECIMALS_OPTION,
  { key: 'endLabels', section: 'options', label: 'Values at line ends', kind: 'toggle', default: true, visibleWhen: overTime },
  { key: 'playback', section: 'time', label: 'While following playback', kind: 'choice', default: 'draw-in', visibleWhen: overTime,
    choices: [{ value: 'draw-in', label: 'Grow to now' }, { value: 'cursor', label: 'Whole campaign' }, { value: 'window', label: 'Recent window' }] },
  { ...AFTER_PLAYHEAD_OPTION, visibleWhen: options => overTime(options) && options.playback === 'cursor' },
  { key: 'windowDays', section: 'time', label: 'Window length', kind: 'number', min: 7, max: 3650, step: 1, unit: 'days', default: 365,
    visibleWhen: options => overTime(options) && options.playback === 'window' },
];

export const definition = {
  kind: 'fuel-branches', version: 1, title: 'Fuel by branch', icon: 'droplet', category: 'Economy',
  description: 'Fuel each military branch requested and received, and when shortages began.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 6, h: 8 },
  countries: { max: Number.MAX_SAFE_INTEGER }, options: OPTIONS,
  variants: choiceVariants(OPTIONS.find(field => field.key === 'view'), () => 'Fuel by branch over time'),
} as const satisfies WidgetMeta;
