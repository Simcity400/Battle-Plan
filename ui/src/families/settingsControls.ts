import type { OptionField, OptionSection, OptionValue } from '../dashboard/options.ts';
import type { TableBase, TableColumnDefaults } from './table/settings.ts';
import { TABLE_SETTINGS_KEY } from './table/settings.ts';

export type BarsBehindMode = 'off' | 'sorted' | 'all';
export type RowHeightMode = 'compact' | 'normal' | 'comfortable';

export const BARS_BEHIND_KEY = 'barsBehind';
export const ROW_HEIGHT_KEY = 'rowHeight';
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

export function barsBehindOf(value: unknown, fallback: BarsBehindMode = 'all'): BarsBehindMode {
  return value === 'off' || value === 'sorted' || value === 'all' ? value : fallback;
}

export function rowHeightOf(value: unknown, fallback: RowHeightMode = 'normal'): RowHeightMode {
  return value === 'compact' || value === 'normal' || value === 'comfortable' ? value : fallback;
}

export function barsBehindControl(
  defaultValue: BarsBehindMode = 'all',
  options?: { key?: string; section?: OptionSection; label?: string; hint?: string; visibleWhen?: OptionField['visibleWhen'] },
): OptionField {
  return {
    key: options?.key ?? BARS_BEHIND_KEY,
    section: options?.section ?? 'options',
    label: options?.label ?? 'Bars behind figures',
    hint: options?.hint,
    kind: 'choice',
    default: defaultValue,
    choices: BARS_BEHIND_CHOICES,
    ...(options?.visibleWhen ? { visibleWhen: options.visibleWhen } : {}),
  };
}

export function rowHeightControl(
  defaultValue: RowHeightMode = 'normal',
  options?: { key?: string; section?: OptionSection; label?: string; hint?: string; visibleWhen?: OptionField['visibleWhen'] },
): OptionField {
  return {
    key: options?.key ?? ROW_HEIGHT_KEY,
    section: options?.section ?? 'options',
    label: options?.label ?? 'Row height',
    hint: options?.hint,
    kind: 'choice',
    default: defaultValue,
    choices: ROW_HEIGHT_CHOICES,
    ...(options?.visibleWhen ? { visibleWhen: options.visibleWhen } : {}),
  };
}

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
    addLabel?: string;
    visibleWhen?: OptionField['visibleWhen'];
  },
): OptionField {
  return {
    key: options?.key ?? REGIONS_KEY,
    section: options?.section ?? 'options',
    label: options?.label ?? 'Regions',
    hint: options?.hint,
    kind: 'ordered-set',
    automatic: options?.automatic ?? 'All regions',
    addLabel: options?.addLabel ?? 'Add region',
    default: '',
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

export function optionValueString(options: Readonly<Record<string, OptionValue>> | undefined, key: string): string | undefined {
  const value = options?.[key];
  return typeof value === 'string' && value.length > 0 ? value : undefined;
}
