import { formatNativeFixed5 } from '../../charts/valueAxis.ts';
import type { OptionField, OptionValue } from '../../dashboard/options.ts';
import type { FamilyTheme } from '../contract.ts';
import type { BarsBehindMode } from '../settingsControls.ts';

export type TableSortDirection = 'ascending' | 'descending';
export type TableDensity = 'compact' | 'normal' | 'comfortable';
export type TableBarsMode = BarsBehindMode;
export type TableTone = 'blue' | 'red' | 'amber' | 'green' | 'grey' | 'country' | 'value-good' | 'value-bad';
export type TableDisplay = 'plain' | 'faint' | 'track' | 'meterAfter' | 'meterBefore' | 'underline' | 'dot' | 'text'
  | 'flagName' | 'name' | 'flag' | 'icon';

export interface TableColumnSetting {
  show: boolean;
  as: TableDisplay;
  tone?: TableTone;
  width: number | null;
  showNames?: boolean;
}

export interface TableSettings {
  density: TableDensity;
  barsBehind?: BarsBehindMode;
  showRank: boolean;
  order: string[];
  columns: Record<string, TableColumnSetting>;
  sort: { column: string; direction: TableSortDirection };
}

export interface TableColumnDefaults {
  key: string;
  label: string;
  as: TableDisplay;
  tone?: TableTone;
  offered: readonly TableDisplay[];
  defaultHidden?: boolean;
  showNamesDefault?: boolean;
  hideable?: boolean;
}

export interface TableBase {
  density: TableDensity;
  barsBehind?: BarsBehindMode;
  showRank?: boolean;
  sort: TableSettings['sort'];
}

export interface TableColumnLayout {
  key: string;
  label: string;
  unit?: string;
  priority: number;
  width: number;
  widest?: readonly string[];
  shown: boolean;
  display: TableDisplay;
  tone?: TableTone;
  numeric?: boolean;
  showNames?: boolean;
  pinned?: boolean;
  barEligible?: boolean;
  notRecordedReason?: string;
}

export interface TableLayout {
  columns: TableColumnLayout[];
  totalWidth: number;
  scrollable: boolean;
  pinnedWidth: number;
  pinnedCount: number;
}

export type TableCanvasContext = CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D;

export type TableTheme = FamilyTheme;

export interface RankMarker {
  delta: number;
  until: number;
}

export interface GlideOffset {
  from: number;
  to: number;
  started: number;
}

export const TABLE_SETTINGS_KEY = 'tableSettings';
export const TABLE_SETTINGS_VERSION = 2;

export const HEADER_HEIGHT = 40;
export const HEADER_TALL_HEIGHT = 52;
export const HEADER_CARET_LANE = 0;
export const ROW_HEIGHT: Readonly<Record<TableDensity, number>> = { compact: 28, normal: 36, comfortable: 44 };
export const MEDIA_ROW_HEIGHT: Readonly<Record<TableDensity, number>> = { compact: 40, normal: 48, comfortable: 58 };
export const CHILD_ROW_HEIGHT = 36;
export const METER_HEIGHT = 6;
export const TRACK_HEIGHT = 6;
export const SORT_GLIDE_MS = 160;
export const RANK_HOLD_MS = 2500;
export const CELL_INSET = 10;
export const ROW_MARGIN_X = 6;
export const COL_GAP = 12;
export const ICON_LANE = 28;
export const DOT_LANE = 12;
export const METER_WIDTH = 56;
export const TRACK_MIN = 56;
export const BAR_GAP = 8;

export const OFFERED = {
  number: ['plain', 'faint', 'track', 'meterAfter', 'meterBefore', 'underline'],
  country: ['flagName', 'name', 'flag'],
  status: ['dot', 'text', 'icon'],
  text: ['plain'],
} as const satisfies Readonly<Record<string, readonly TableDisplay[]>>;

export const isCountryColumn = (column: Pick<TableColumnDefaults, 'as' | 'offered'>): boolean =>
  column.as === 'flag' || column.as === 'flagName' || column.offered.includes('flag') || column.offered.includes('flagName');

export function displayExtra(column: Pick<TableColumnLayout, 'display'>): number {
  switch (column.display) {
    case 'faint': return 8;
    case 'track': return TRACK_MIN + BAR_GAP;
    case 'meterAfter': case 'meterBefore': return METER_WIDTH + BAR_GAP;
    case 'dot': return DOT_LANE;
    case 'flagName': case 'icon': return ICON_LANE;
    case 'flag': return 20;
    default: return 0;
  }
}

export const measuresColumn = (column: Pick<TableColumnLayout, 'display' | 'tone' | 'barEligible'>): boolean =>
  column.barEligible === true
  || column.display === 'faint' || column.display === 'track'
  || column.display === 'meterAfter' || column.display === 'meterBefore'
  || column.display === 'underline' || column.tone === 'value-good' || column.tone === 'value-bad';

export const showsIcon = (display: TableDisplay): boolean =>
  display === 'flagName' || display === 'flag' || display === 'icon';

export const endAligned = (column: Pick<TableColumnLayout, 'numeric' | 'display'>): boolean =>
  !!column.numeric;

export function scrollBounds(layout: TableLayout, viewport: number, scrollLeft: number): { max: number; left: number } {
  const max = Math.max(0, layout.totalWidth - viewport);
  return { max, left: Math.max(0, Math.min(max, scrollLeft)) };
}

export function glideOffset(glide: GlideOffset | undefined, now: number, duration = SORT_GLIDE_MS): number {
  if (!glide) return 0;
  const t = Math.max(0, Math.min(1, (now - glide.started) / duration));
  const eased = t * t * (3 - 2 * t);
  return glide.from + (glide.to - glide.from) * eased;
}

export function rankMarkers(
  previous: readonly string[],
  current: readonly string[],
  now: number,
  held: ReadonlyMap<string, RankMarker>,
  holdMs = RANK_HOLD_MS,
): Map<string, RankMarker> {
  const before = new Map<string, number>();
  previous.forEach((key, index) => before.set(key, index));
  const next = new Map<string, RankMarker>();
  current.forEach((key, index) => {
    const prev = before.get(key);
    if (prev !== undefined && prev !== index) {
      next.set(key, { delta: prev - index, until: now + holdMs });
    } else {
      const existing = held.get(key);
      if (existing && existing.until > now) next.set(key, existing);
    }
  });
  return next;
}

export function percentCell(raw: string | bigint): string {
  const value = BigInt(raw);
  const rounded = BigInt.asIntN(32, (value + (value < 0n ? -50000n : 50000n)) / 100000n);
  const clamped = rounded < 0n ? 0n : rounded > 100n ? 100n : rounded;
  return `${formatNativeFixed5(clamped * 100000n, 0)}%`;
}

export function amountCell(raw: string | bigint): string {
  const amount = BigInt(raw);
  return amount === 0n ? '-' : amount < 0n ? '!' : formatNativeFixed5(amount, 2);
}

export function strengthBar(raw: string | bigint): bigint {
  const percent = BigInt.asIntN(64, BigInt(raw) * 100n);
  const biased = BigInt.asIntN(64, percent + (percent < 0n ? -50000n : 50000n));
  return BigInt.asIntN(32, biased / 100000n) * 1000n;
}

export function tableFamilyOptions(columns: readonly TableColumnDefaults[], base: TableBase): readonly OptionField[] {
  return [{ key: TABLE_SETTINGS_KEY, section: 'options', label: 'Table', kind: 'table', default: '', columns, base }];
}

const columnDefault = (column: TableColumnDefaults): TableColumnSetting => {
  const country = isCountryColumn(column);
  const showNames = country ? (column.showNamesDefault ?? (column.as === 'flagName')) : undefined;
  return {
    show: column.defaultHidden !== true,
    as: country ? (showNames ? 'flagName' : 'flag') : column.as,
    ...(column.tone && { tone: column.tone }),
    width: null,
    ...(showNames !== undefined && { showNames }),
  };
};

export function tableDefaults(columns: readonly TableColumnDefaults[], base: TableBase): TableSettings {
  return {
    density: base.density ?? 'normal',
    barsBehind: base.barsBehind ?? 'all',
    showRank: base.showRank ?? false,
    order: columns.map(column => column.key),
    columns: Object.fromEntries(columns.map(column => [column.key, columnDefault(column)])),
    sort: { ...base.sort },
  };
}

const displays = new Set<TableDisplay>([
  'plain', 'faint', 'track', 'meterAfter', 'meterBefore', 'underline',
  'dot', 'text', 'flagName', 'name', 'flag', 'icon',
]);
const tones = new Set<TableTone>(['blue', 'red', 'amber', 'green', 'grey', 'country', 'value-good', 'value-bad']);
const densities = new Set<TableDensity>(['compact', 'normal', 'comfortable']);
const barsBehindModes = new Set<BarsBehindMode>(['off', 'sorted', 'all']);
const own = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
const validWidth = (width: unknown): width is number | null => width === null || typeof width === 'number' && Number.isFinite(width) && width >= 40 && width <= 1000;
const validTone = (tone: unknown): tone is TableTone | undefined => tone === undefined || typeof tone === 'string' && tones.has(tone as TableTone);

function keptEntry(stored: Record<string, unknown>): TableColumnSetting | null {
  if (typeof stored.show !== 'boolean' || typeof stored.as !== 'string' || !displays.has(stored.as as TableDisplay) || !validTone(stored.tone) || !validWidth(stored.width)) return null;
  const showNames = typeof stored.showNames === 'boolean' ? stored.showNames : undefined;
  return {
    show: stored.show,
    as: stored.as as TableDisplay,
    ...(stored.tone !== undefined && { tone: stored.tone }),
    width: stored.width,
    ...(showNames !== undefined && { showNames }),
  };
}

export function tableSettingsOf(value: unknown, columns: readonly TableColumnDefaults[], base: TableBase): TableSettings {
  const result = tableDefaults(columns, base);
  if (typeof value !== 'string' || !value) return result;
  let parsed: unknown;
  try { parsed = JSON.parse(value); } catch { return result; }
  if (!own(parsed) || parsed.v !== TABLE_SETTINGS_VERSION
    || typeof parsed.density !== 'string' || !densities.has(parsed.density as TableDensity)
    || !Array.isArray(parsed.order) || !own(parsed.columns) || !own(parsed.sort)
    || typeof parsed.sort.column !== 'string' || !['ascending', 'descending'].includes(String(parsed.sort.direction))) return result;
  result.density = parsed.density as TableDensity;
  if (typeof parsed.barsBehind === 'string' && barsBehindModes.has(parsed.barsBehind as BarsBehindMode)) {
    result.barsBehind = parsed.barsBehind as BarsBehindMode;
  }
  result.showRank = typeof parsed.showRank === 'boolean' ? parsed.showRank : (base.showRank ?? false);
  const known = new Map(columns.map(column => [column.key, column]));
  for (const [key, stored] of Object.entries(parsed.columns)) {
    if (!own(stored)) continue;
    const column = known.get(key);
    if (!column) {
      const kept = keptEntry(stored);
      if (kept) result.columns[key] = kept;
      continue;
    }
    const entry = result.columns[key]!;
    if (typeof stored.show === 'boolean') entry.show = stored.show;
    if (typeof stored.as === 'string' && column.offered.includes(stored.as as TableDisplay)) entry.as = stored.as as TableDisplay;
    if (typeof stored.showNames === 'boolean' && isCountryColumn(column)) {
      entry.showNames = stored.showNames;
      entry.as = stored.showNames ? 'flagName' : 'flag';
    }
    if (validTone(stored.tone)) {
      if (stored.tone === undefined) delete entry.tone;
      else entry.tone = stored.tone;
    }
    if (validWidth(stored.width)) entry.width = stored.width;
  }
  const seen = new Set<string>();
  result.order = parsed.order.filter((key): key is string => {
    if (typeof key !== 'string' || !(key in result.columns) || seen.has(key)) return false;
    seen.add(key);
    return true;
  });
  for (const column of columns) if (!seen.has(column.key)) result.order.push(column.key);
  if (known.has(parsed.sort.column)) result.sort = { column: parsed.sort.column, direction: parsed.sort.direction as 'ascending' | 'descending' };
  return result;
}

export function encodeTableSettings(settings: TableSettings): string {
  return JSON.stringify({
    v: TABLE_SETTINGS_VERSION,
    density: settings.density,
    ...(settings.barsBehind !== undefined ? { barsBehind: settings.barsBehind } : {}),
    showRank: settings.showRank,
    order: settings.order,
    columns: settings.columns,
    sort: settings.sort,
  });
}

export function tableSettingValue(settings: TableSettings): OptionValue {
  return encodeTableSettings(settings);
}

export function cloneTableSettings(settings: TableSettings): TableSettings {
  return {
    density: settings.density,
    ...(settings.barsBehind !== undefined ? { barsBehind: settings.barsBehind } : {}),
    showRank: settings.showRank,
    order: [...settings.order],
    columns: Object.fromEntries(Object.entries(settings.columns).map(([key, value]) => [key, { ...value }])),
    sort: { ...settings.sort },
  };
}

export function updateTableSettings(settings: TableSettings, update: (next: TableSettings) => void): TableSettings {
  const next = cloneTableSettings(settings);
  update(next);
  return next;
}
