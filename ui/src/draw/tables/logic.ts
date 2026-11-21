import { formatNativeFixed5 } from '../../charts/valueAxis.ts';

export type TableSortDirection = 'ascending' | 'descending';
export type TableOverflow = 'tooltip' | 'scroll';
export type TableDensity = 'comfortable' | 'compact';
export type TableTone = 'blue' | 'red' | 'amber' | 'green' | 'grey' | 'country' | 'value-good' | 'value-bad';
export type TableFormat = 'fixed5Percent' | 'fixed5Amount' | 'integer' | { decimals: number; percent?: boolean; zero?: string };
export type TableDisplay = 'plain' | 'faint' | 'track' | 'meterAfter' | 'meterBefore' | 'underline' | 'dot' | 'outline' | 'filled' | 'text'
  | 'flagName' | 'name' | 'flag' | 'icon';

export interface TableSettings {
  density: TableDensity;
  overflow: TableOverflow;
  order: string[];
  columns: Record<string, { show: boolean; as: TableDisplay; tone?: TableTone; width: number | null }>;
  sort: { column: string; direction: TableSortDirection };
}

export interface TableColumnLayout {
  key: string;
  label: string;
  priority: number;
  width: number;
  widest?: readonly string[];
  shown: boolean;
  display: TableDisplay;
  tone?: TableTone;
  numeric?: boolean;
  format?: TableFormat;
}

export interface TableLayoutInput {
  columns: readonly TableColumnLayout[];
  available: number;
  rankWidth?: number;
  rankShown?: boolean;
}

export interface TableLayout {
  columns: TableColumnLayout[];
  hidden: string[];
  totalWidth: number;
  scrollable: boolean;
  pinnedWidth: number;
}

export const HEADER_CARET_LANE = 28;
export const ROW_HEIGHT = { comfortable: 28, compact: 22 } as const;
export const METER_HEIGHT = 4;
export const TRACK_HEIGHT = 6;
export const SORT_GLIDE_MS = 160;
export const RANK_HOLD_MS = 2500;
export const CELL_INSET = 8;
export const ICON_LANE = 26;
export const DOT_LANE = 12;
export const METER_WIDTH = 44;
export const TRACK_MIN = 32;
export const BAR_GAP = 8;

export function displayExtra(column: Pick<TableColumnLayout, 'display'>): number {
  switch (column.display) {
    case 'faint': return 4;
    case 'track': return TRACK_MIN + BAR_GAP;
    case 'meterAfter': case 'meterBefore': return METER_WIDTH + BAR_GAP;
    case 'dot': return DOT_LANE;
    case 'outline': case 'filled': return 16;
    case 'flagName': case 'icon': return ICON_LANE;
    case 'flag': return 20;
    default: return 0;
  }
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

export function amountText(value: number | bigint, decimals: number, percent = false): string {
  let raw: number | bigint = typeof value === 'number' ? Math.trunc(value) : value;
  if (percent) raw = typeof raw === 'number' && Math.abs(raw) < 2 ** 46 ? raw * 100 : BigInt(raw) * 100n;
  const negative = raw < 0;
  const digits = String(negative ? -raw : raw).padStart(decimals + 1, '0');
  const whole = digits.slice(0, digits.length - decimals).replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  const fraction = decimals ? digits.slice(digits.length - decimals).replace(/0+$/, '') : '';
  return `${negative ? '−' : ''}${whole}${fraction ? `.${fraction}` : ''}${percent ? '%' : ''}`;
}

export function formatTableNumber(value: number | bigint, format?: TableFormat): string {
  if (typeof format === 'object') return format.zero !== undefined && (value === 0 || value === 0n) ? format.zero : amountText(value, format.decimals, format.percent);
  if (typeof value === 'bigint' || Math.abs(value) >= 2 ** 40) {
    const big = typeof value === 'bigint' ? value : BigInt(Math.trunc(value));
    return format === 'fixed5Percent' ? percentCell(big) : format === 'fixed5Amount' ? amountCell(big) : big.toString();
  }
  const raw = Math.trunc(value);
  if (format === 'fixed5Percent') return `${Math.min(100, Math.max(0, Math.trunc((raw + (raw < 0 ? -50000 : 50000)) / 100000)))}%`;
  if (format === 'fixed5Amount') {
    if (raw <= 0) return raw === 0 ? '-' : '!';
    return `${Math.trunc(raw / 100000)}.${String(raw % 100000).padStart(5, '0').slice(0, 2)}`;
  }
  return String(raw);
}

export const measuresColumn = (column: Pick<TableColumnLayout, 'display' | 'tone'>) => column.display === 'faint' || column.display === 'track'
  || column.display === 'meterAfter' || column.display === 'meterBefore' || column.display === 'underline' || column.tone === 'value-good' || column.tone === 'value-bad';

export const showsIcon = (display: TableDisplay) => display === 'flagName' || display === 'flag' || display === 'icon';

export const endAligned = (column: Pick<TableColumnLayout, 'numeric' | 'display'>) => !!column.numeric && column.display !== 'track' && column.display !== 'meterBefore';

export function layoutTable(input: TableLayoutInput, overflow: TableOverflow): TableLayout {
  const shown = input.columns.filter(column => column.shown);
  const rank = input.rankShown ? input.rankWidth ?? 28 : 0;
  const total = () => rank + shown.reduce((sum, column) => sum + column.width, 0);
  if (overflow === 'scroll' || total() <= input.available) {
    return { columns: shown, hidden: [], totalWidth: total(), scrollable: overflow === 'scroll' && total() > input.available,
      pinnedWidth: (input.rankShown ? rank : 0) + (shown[0]?.width ?? 0) };
  }
  const hidden = new Set<string>();
  const candidates = shown.slice().sort((a, b) => b.priority - a.priority);
  for (const column of candidates) {
    if (column === shown[0] || total() <= input.available) continue;
    hidden.add(column.key);
    const visible = shown.filter(item => !hidden.has(item.key));
    shown.splice(0, shown.length, ...visible);
  }
  return { columns: shown, hidden: [...hidden], totalWidth: total(), scrollable: false,
    pinnedWidth: (input.rankShown ? rank : 0) + (shown[0]?.width ?? 0) };
}

export interface TableRowLike { key: string; revealAt?: number; values?: Readonly<Record<string, string | number | bigint | null>> }
function missing(value: unknown): boolean {
  return value === null || value === undefined || typeof value === 'number' && !Number.isFinite(value);
}
const collator = new Intl.Collator(undefined, { numeric: true });
export function compareSortKeys(a: unknown, b: unknown, sign: number): number {
  const am = missing(a), bm = missing(b);
  if (am || bm) return am === bm ? 0 : am ? 1 : -1;
  const order = typeof a === 'string' || typeof b === 'string' ? collator.compare(String(a), String(b))
    : (a as number | bigint) < (b as number | bigint) ? -1 : (a as number | bigint) > (b as number | bigint) ? 1 : 0;
  return sign * order;
}

export function sortTableRows<T extends TableRowLike>(rows: readonly T[], column: string, direction: TableSortDirection): T[] {
  const sign = direction === 'ascending' ? 1 : -1;
  return rows.map((row, index) => ({ row, index, value: row.values?.[column] }))
    .sort((a, b) => compareSortKeys(a.value, b.value, sign) || a.index - b.index).map(item => item.row);
}

export function revealedRows<T extends { revealAt?: number }>(rows: readonly T[], hour: number): T[] {
  return rows.filter(row => row.revealAt === undefined || row.revealAt <= hour);
}

export function preserveRevealTimes<T extends { key: string; revealAt?: number }>(rows: T[], prior: readonly T[]): T[] {
  if (rows === prior) return rows;
  const times = new Map<string, number>();
  for (const row of prior) if (row.revealAt !== undefined) times.set(row.key, row.revealAt);
  if (!times.size) return rows;
  let changed = false;
  const kept = rows.map(row => {
    const revealAt = times.get(row.key);
    if (revealAt === undefined || revealAt === row.revealAt) return row;
    changed = true;
    return { ...row, revealAt };
  });
  return changed ? kept : rows;
}

export interface GlideOffset { from: number; to: number; started: number }
export function glideOffset(glide: GlideOffset | undefined, now: number, duration = SORT_GLIDE_MS): number {
  if (!glide) return 0;
  const t = Math.max(0, Math.min(1, (now - glide.started) / duration));
  const eased = t * t * (3 - 2 * t);
  return glide.from + (glide.to - glide.from) * eased;
}

export interface TableHit { kind: 'row' | 'header' | 'column-menu' | 'resize' | 'empty'; rowKey?: string; column?: string }
export function hitTable(layout: TableLayout, rows: readonly TableRowLike[], point: { x: number; y: number }, rowHeight: number,
  headerHeight: number, scrollLeft: number, rankShown = false, rankWidth = 28, scrollTop = 0): TableHit {
  const firstWidth = layout.columns[0]?.width ?? 0;
  const pinned = layout.scrollable && (rankShown && point.x < rankWidth || point.x >= (rankShown ? rankWidth : 0) && point.x < (rankShown ? rankWidth : 0) + firstWidth);
  const bodyX = pinned ? point.x : point.x + scrollLeft;
  if (point.y < headerHeight) {
    let x = rankShown ? rankWidth - (layout.scrollable ? scrollLeft : 0) : -(layout.scrollable ? scrollLeft : 0);
    if (rankShown && point.x < rankWidth) return { kind: 'header' };
    for (const column of layout.columns) {
      if (layout.scrollable && column === layout.columns[0]) x = rankShown ? rankWidth : 0;
      if (point.x >= x && point.x <= x + column.width) {
        return { kind: point.x >= x + column.width - 4 ? 'resize' : point.x >= x + column.width - HEADER_CARET_LANE ? 'column-menu' : 'header', column: column.key };
      }
      x += column.width;
      if (layout.scrollable && column === layout.columns[0]) x -= scrollLeft;
    }
    return { kind: 'empty' };
  }
  const index = Math.floor((point.y + scrollTop - headerHeight) / rowHeight);
  const row = rows[index];
  if (!row) return { kind: 'empty' };
  let x = 0;
  if (rankShown) {
    if (bodyX < rankWidth) return { kind: 'row', rowKey: row.key };
    x += rankWidth;
  }
  for (const column of layout.columns) {
    if (bodyX >= x && bodyX < x + column.width) return { kind: 'row', rowKey: row.key, column: column.key };
    x += column.width;
  }
  return { kind: 'row', rowKey: row.key };
}

export function scrollBounds(layout: TableLayout, viewport: number, scrollLeft: number): { max: number; left: number } {
  const max = Math.max(0, layout.totalWidth - viewport);
  return { max, left: Math.max(0, Math.min(max, scrollLeft)) };
}
