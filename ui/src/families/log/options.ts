import type { LogDensity } from './model.ts';
import type { OptionField, OptionValue } from '../../dashboard/options';

export const LOG_FAMILY_OPTIONS: readonly OptionField[] = [
  {
    key: 'density',
    section: 'options',
    label: 'Density',
    kind: 'choice',
    default: 'comfortable',
    choices: [
      { value: 'comfortable', label: 'Comfortable' },
      { value: 'compact', label: 'Compact' },
    ],
  },
  { key: 'months', section: 'options', label: 'Month headers', kind: 'toggle', default: false },
  { key: 'chips', section: 'options', label: 'Filter chips', kind: 'toggle', default: true },
];

export interface LogFamilySettings {
  readonly density: LogDensity;
  readonly months: boolean;
  readonly chips: boolean;
}

export function logFamilySettings(options: Readonly<Record<string, OptionValue>>): LogFamilySettings {
  return {
    density: options.density === 'compact' ? 'compact' : 'comfortable',
    months: options.months === true,
    chips: options.chips !== false,
  };
}
