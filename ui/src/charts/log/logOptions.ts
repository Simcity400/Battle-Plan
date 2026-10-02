/** The settings every log offers (spec 2.2d, mock-up style-5): density, month
 * dividers, the new-row animation and the filter chips above it. A log widget
 * lists `LOG_OPTIONS` among its own and reads them with `logSettings`. */
import type { OptionField, OptionValue } from '../../dashboard/options';
import type { LogDensity } from './logData';

export const LOG_OPTIONS: readonly OptionField[] = [
  { key: 'density', section: 'options', label: 'Density', kind: 'choice', default: 'comfortable',
    choices: [{ value: 'comfortable', label: 'Comfortable' }, { value: 'compact', label: 'Compact' }] },
  { key: 'months', section: 'options', label: 'Month headers', hint: 'A divider before each month', kind: 'toggle', default: false },
  { key: 'rowAnimation', section: 'options', label: 'New-row animation', hint: 'New rows slide in with a brief highlight while following playback', kind: 'toggle', default: true },
  { key: 'chips', section: 'options', label: 'Filter chips', hint: 'Chips above the log to filter it', kind: 'toggle', default: true },
];

export interface LogSettings { density: LogDensity; months: boolean; animate: boolean; chips: boolean }

/** The log settings from resolved options (`resolveOptions`). */
export function logSettings(options: Readonly<Record<string, OptionValue>>): LogSettings {
  return {
    density: options.density === 'compact' ? 'compact' : 'comfortable',
    months: options.months === true,
    animate: options.rowAnimation !== false,
    chips: options.chips !== false,
  };
}
