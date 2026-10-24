import type { WidgetMeta } from '../../dashboard/definitions';
import { AFTER_PLAYHEAD_OPTION } from '../../dashboard/options.ts';
import type { OptionField } from '../../dashboard/options';
import { breakdownOption, DRILL_OPTION, breakdownVariants } from '../../charts/drill/drillState.ts';
import { FUEL_DECIMALS_OPTION } from './fuelData.ts';

export const BREAKDOWNS = [{ id: 'gain', label: 'Fuel gained' }, { id: 'burn', label: 'Fuel burned' },
  { id: 'consumption', label: 'Individual current consumption' }] as const;

export const OPTIONS: readonly OptionField[] = [
  breakdownOption(BREAKDOWNS, 'Fuel level'),
  DRILL_OPTION,
  { key: 'fuelCountry', section: 'options', label: 'Selected fuel country', kind: 'content-choice',
    automatic: 'The widget’s country selection', default: '', visibleWhen: () => false },
  { key: 'fuelRecipient', section: 'options', label: 'Selected fuel recipient', kind: 'content-choice',
    automatic: 'Choose a consumer and recipient', default: '', visibleWhen: () => false },
  { key: 'fuelConsumption', section: 'options', label: 'Selected current consumer', kind: 'content-choice',
    automatic: 'All current consumers', default: '', visibleWhen: () => false },
  { key: 'fuelConsumptionLevel', section: 'options', label: 'Current consumption level', kind: 'content-choice',
    automatic: 'All current consumers', default: '', visibleWhen: () => false },
  { key: 'fuelExtent', section: 'options', label: 'Fuel detail history', kind: 'content-choice',
    automatic: 'The widget’s time range', default: '', visibleWhen: () => false },
  { key: 'playback', section: 'time', label: 'While following playback', kind: 'choice', default: 'draw-in',
    choices: [{ value: 'draw-in', label: 'Grow to now' }, { value: 'cursor', label: 'Whole campaign' }, { value: 'window', label: 'Recent window' }] },
  AFTER_PLAYHEAD_OPTION,
  { key: 'windowDays', section: 'time', label: 'Window length', kind: 'number', min: 7, max: 3650, step: 1, unit: 'days', default: 365,
    visibleWhen: options => options.playback === 'window' },
  { key: 'strip', section: 'options', label: 'Gain and burn bars', kind: 'toggle', default: true },
  { key: 'capacity', section: 'options', label: 'Capacity', kind: 'toggle', default: true },
  { key: 'areaFill', section: 'options', label: 'Area fill', kind: 'toggle', default: true },
  { key: 'endLabels', section: 'options', label: 'Values at line ends', kind: 'toggle', default: true },
  FUEL_DECIMALS_OPTION,
  { key: 'yMin', section: 'axes', label: 'Value axis from', hint: 'Empty starts at zero', kind: 'number', min: -1e9, max: 1e9, step: 1, default: null },
  { key: 'yMax', section: 'axes', label: 'Value axis to', hint: 'Empty fits the data', kind: 'number', min: -1e9, max: 1e9, step: 1, default: null },
  { key: 'log', section: 'axes', label: 'Log scale', hint: 'Only when every value shown is positive', kind: 'toggle', default: false },
];

export const definition = {
  kind: 'fuel', version: 1, title: 'Fuel', icon: 'droplet',
  description: 'A country’s fuel against its capacity over time, with each day’s gain and burn.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 12, h: 10 },
  countries: { max: Number.MAX_SAFE_INTEGER }, options: OPTIONS,
  variants: breakdownVariants('Fuel', OPTIONS),
} as const satisfies WidgetMeta;
