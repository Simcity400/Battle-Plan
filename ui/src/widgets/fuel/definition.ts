import type { WidgetMeta } from '../../dashboard/definitions';
import { AFTER_PLAYHEAD_OPTION } from '../../dashboard/options.ts';
import type { OptionField } from '../../dashboard/options';
import { FUEL_DECIMALS_OPTION } from './fuelData.ts';

export const BREAKDOWNS = [{ id: 'gain', label: 'Fuel gained' }, { id: 'burn', label: 'Fuel burned' },
  { id: 'consumption', label: 'Individual current consumption' }] as const;

export const OPTIONS: readonly OptionField[] = [
  { key: 'playback', section: 'time', label: 'While following playback', kind: 'choice', default: 'draw-in',
    choices: [{ value: 'draw-in', label: 'Grow to now' }, { value: 'cursor', label: 'Whole campaign' }, { value: 'window', label: 'Recent window' }] },
  AFTER_PLAYHEAD_OPTION,
  { key: 'windowDays', section: 'time', label: 'Window length', kind: 'number', min: 7, max: 3650, step: 1, unit: 'days', default: 365,
    visibleWhen: options => options.playback === 'window' },
  { key: 'capacity', section: 'options', label: 'Capacity', kind: 'toggle', default: true },
  { key: 'areaFill', section: 'options', label: 'Area fill', kind: 'toggle', default: true },
  FUEL_DECIMALS_OPTION,
];

export const definition = {
  kind: 'fuel', version: 1, title: 'Fuel', icon: 'chart-line',
  description: 'A country’s fuel against its capacity over time; a day opens its gain, burn and consumption.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 12, h: 10 },
  countries: { max: 1 }, options: OPTIONS,
} as const satisfies WidgetMeta;
