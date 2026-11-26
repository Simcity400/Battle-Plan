import type { OptionField, OptionSection } from '../dashboard/options.ts';
import type { TableBase, TableColumnDefaults } from './table/settings.ts';
import { TABLE_SETTINGS_KEY } from './table/settings.ts';

export type BarsBehindMode = 'off' | 'sorted' | 'all';

export const REGIONS_KEY = 'regions';

export const BARS_BEHIND_CHOICES = [
  { value: 'all', label: 'Every column' },
  { value: 'sorted', label: 'Sorted column' },
  { value: 'off', label: 'Off' },
] as const;

export const ROW_HEIGHT_CHOICES = [
  { value: 'compact', label: 'Compact' },
  { value: 'normal', label: 'Default' },
  { value: 'comfortable', label: 'Comfortable' },
] as const;

export function columnsControl(
  columns: readonly TableColumnDefaults[],
  base: TableBase,
  options?: { key?: string; section?: OptionSection; label?: string; hint?: string; visibleWhen?: OptionField['visibleWhen'] },
): OptionField {
  return {
    key: options?.key ?? TABLE_SETTINGS_KEY,
    section: options?.section ?? 'options',
    label: options?.label ?? 'Table',
    hint: options?.hint,
    kind: 'table',
    default: '',
    columns,
    base,
    ...(options?.visibleWhen ? { visibleWhen: options.visibleWhen } : {}),
  };
}

export function regionPickerControl(
  options?: {
    key?: string;
    section?: OptionSection;
    label?: string;
    automatic?: string;
    hint?: string;
    sea?: boolean;
    visibleWhen?: OptionField['visibleWhen'];
  },
): OptionField {
  return {
    key: options?.key ?? REGIONS_KEY,
    section: options?.section ?? 'options',
    label: options?.label ?? 'Regions',
    hint: options?.hint,
    kind: 'regions',
    automatic: options?.automatic ?? 'Most active regions',
    default: '',
    ...(options?.sea ? { sea: true } : {}),
    ...(options?.visibleWhen ? { visibleWhen: options.visibleWhen } : {}),
  };
}

export function viewSwitchControl(
  choices: readonly { value: string; label: string }[],
  defaultView?: string,
  options?: { key?: string; label?: string; hint?: string },
): OptionField {
  const first = defaultView ?? choices[0]?.value ?? 'default';
  return {
    key: options?.key ?? 'view',
    section: 'options',
    label: options?.label ?? 'View',
    hint: options?.hint,
    kind: 'choice',
    default: first,
    choices,
  };
}
