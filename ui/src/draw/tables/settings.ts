import type { OptionField, OptionValue } from '../../dashboard/options.ts';
import type { TableDensity, TableDisplay, TableOverflow, TableSettings, TableTone } from './logic.ts';

export const TABLE_SETTINGS_KEY = 'tableSettings';
export const TABLE_SETTINGS_VERSION = 1;

export interface TableColumnDefaults { key: string; label: string; as: TableDisplay; tone?: TableTone; offered: readonly TableDisplay[] }
export interface TableBase { density: TableDensity; overflow: TableOverflow; sort: TableSettings['sort'] }

export const OFFERED = {
  number: ['plain', 'faint', 'track', 'meterAfter', 'meterBefore', 'underline'],
  country: ['flagName', 'name', 'flag'],
  status: ['dot', 'outline', 'filled', 'text', 'icon'],
  text: ['plain'],
} as const satisfies Readonly<Record<string, readonly TableDisplay[]>>;

export function tableFamilyOptions(columns: readonly TableColumnDefaults[], base: TableBase): readonly OptionField[] {
  return [{ key: TABLE_SETTINGS_KEY, section: 'options', label: 'Table', kind: 'table', default: '', columns, base }];
}

type ColumnSetting = TableSettings['columns'][string];
const columnDefault = (column: TableColumnDefaults): ColumnSetting => ({ show: true, as: column.as, ...(column.tone && { tone: column.tone }), width: null });

export function tableDefaults(columns: readonly TableColumnDefaults[], base: TableBase): TableSettings {
  return { density: base.density, overflow: base.overflow, order: columns.map(column => column.key),
    columns: Object.fromEntries(columns.map(column => [column.key, columnDefault(column)])), sort: { ...base.sort } };
}

const displays = new Set<TableDisplay>(['plain', 'faint', 'track', 'meterAfter', 'meterBefore', 'underline', 'dot', 'outline', 'filled', 'text',
  'flagName', 'name', 'flag', 'icon']);
const tones = new Set<TableTone>(['blue', 'red', 'amber', 'green', 'grey', 'country', 'value-good', 'value-bad']);
const own = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
const validWidth = (width: unknown): width is number | null => width === null || typeof width === 'number' && Number.isFinite(width) && width >= 40 && width <= 1000;
const validTone = (tone: unknown): tone is TableTone | undefined => tone === undefined || typeof tone === 'string' && tones.has(tone as TableTone);

function keptEntry(stored: Record<string, unknown>): ColumnSetting | null {
  if (typeof stored.show !== 'boolean' || typeof stored.as !== 'string' || !displays.has(stored.as as TableDisplay) || !validTone(stored.tone) || !validWidth(stored.width)) return null;
  return { show: stored.show, as: stored.as as TableDisplay, ...(stored.tone !== undefined && { tone: stored.tone }), width: stored.width };
}

export function tableSettingsOf(value: unknown, columns: readonly TableColumnDefaults[], base: TableBase): TableSettings {
  const result = tableDefaults(columns, base);
  if (typeof value !== 'string' || !value) return result;
  let parsed: unknown;
  try { parsed = JSON.parse(value); } catch { return result; }
  if (!own(parsed) || parsed.v !== TABLE_SETTINGS_VERSION || (parsed.density !== 'comfortable' && parsed.density !== 'compact')
    || (parsed.overflow !== 'tooltip' && parsed.overflow !== 'scroll') || !Array.isArray(parsed.order) || !own(parsed.columns) || !own(parsed.sort)
    || typeof parsed.sort.column !== 'string' || !['ascending', 'descending'].includes(String(parsed.sort.direction))) return result;
  result.density = parsed.density;
  result.overflow = parsed.overflow;
  const known = new Map(columns.map(column => [column.key, column]));
  for (const [key, stored] of Object.entries(parsed.columns)) {
    if (!own(stored)) continue;
    const column = known.get(key);
    if (!column) { const kept = keptEntry(stored); if (kept) result.columns[key] = kept; continue; }
    const entry = result.columns[key]!;
    if (typeof stored.show === 'boolean') entry.show = stored.show;
    if (typeof stored.as === 'string' && column.offered.includes(stored.as as TableDisplay)) entry.as = stored.as as TableDisplay;
    if (validTone(stored.tone)) { if (stored.tone === undefined) delete entry.tone; else entry.tone = stored.tone; }
    if (validWidth(stored.width)) entry.width = stored.width;
  }
  const seen = new Set<string>();
  result.order = parsed.order.filter((key): key is string => {
    if (typeof key !== 'string' || !(key in result.columns) || seen.has(key)) return false;
    seen.add(key); return true;
  });
  for (const column of columns) if (!seen.has(column.key)) result.order.push(column.key);
  if (known.has(parsed.sort.column)) result.sort = { column: parsed.sort.column, direction: parsed.sort.direction as 'ascending' | 'descending' };
  return result;
}

export function encodeTableSettings(settings: TableSettings): string {
  return JSON.stringify({ v: TABLE_SETTINGS_VERSION, density: settings.density, overflow: settings.overflow, order: settings.order,
    columns: settings.columns, sort: settings.sort });
}

export function tableSettingValue(settings: TableSettings): OptionValue { return encodeTableSettings(settings); }

export function cloneTableSettings(settings: TableSettings): TableSettings {
  return { density: settings.density, overflow: settings.overflow, order: [...settings.order], columns: Object.fromEntries(Object.entries(settings.columns).map(([key, value]) => [key, { ...value }])), sort: { ...settings.sort } };
}

export function updateTableSettings(settings: TableSettings, update: (next: TableSettings) => void): TableSettings {
  const next = cloneTableSettings(settings); update(next); return next;
}
