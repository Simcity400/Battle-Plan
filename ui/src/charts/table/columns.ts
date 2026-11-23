import type { ReactNode } from 'react';
import { liveKey } from '../live.ts';
import type { LiveValues } from '../live';
import { usNumber } from '../numberFormat.ts';
import { tooltipHeight } from '../tooltip.ts';
import type { TooltipContent, TooltipRow } from '../tooltip';
import { isMissing } from './logic.ts';
import type { SortValue, SortDirection } from './logic';

export interface TableColumn<T> {
  key: string;
  header: ReactNode;
  headerRevision?: unknown;
  label: string;
  hint?: string;
  value: (row: T) => SortValue;
  render?: (row: T) => ReactNode;
  paint?: (value: NonNullable<SortValue>, key: string) => string;
  sortsLive?: boolean;
  missingText?: (key: string, reason: string) => string;
  barOf?: (row: T) => number | bigint | null | 'cell';
  barCell?: { id: string; value?: (value: number) => number | bigint; tone?: (value: number) => string | undefined };
  cellClass?: (row: T) => string | undefined;
  widthKey?: (row: T) => string | readonly unknown[];
  renderMissing?: boolean;
  align?: 'start' | 'end';
  numeric?: boolean;
  sortable?: boolean;
  direction?: SortDirection;
  bar?: 'visible' | number | bigint;
  width?: string;
  priority?: number;
  text?: (row: T) => string | null;
  inTooltip?: boolean;
}

export function hideOrder<T>(columns: readonly TableColumn<T>[]): string[] {
  return columns.map((column, index) => ({ column, index }))
    .filter(({ column }) => column.priority != null && Number.isFinite(column.priority))
    .sort((a, b) => b.column.priority! - a.column.priority! || b.index - a.index)
    .map(({ column }) => column.key);
}

export function cellText<T>(column: TableColumn<T>, row: T): string {
  if (column.text) return column.text(row) ?? '—';
  const value = column.value(row);
  if (isMissing(value) && !column.renderMissing) return '—';
  const shown = column.render?.(row);
  if (typeof shown === 'string') return shown;
  if (typeof shown === 'number' || typeof shown === 'bigint') return usNumber.format(shown);
  if (typeof value === 'string') return value;
  if (typeof value === 'number' || typeof value === 'bigint') return usNumber.format(value);
  return '—';
}

export function tableGrid<T>(columns: readonly Pick<TableColumn<T>, 'key' | 'width'>[], rank: boolean, widths: ReadonlyMap<string, number>): string {
  return [rank ? 'var(--table-rank-width)' : '', ...columns.map(column => column.width === 'max-content'
    ? widths.has(column.key) ? `${widths.get(column.key)}px` : 'max-content'
    : column.width ?? 'minmax(100px, 1fr)')].filter(Boolean).join(' ');
}

export function growColumnWidths(before: ReadonlyMap<string, number>, readings: readonly { key: string; width: number }[]): ReadonlyMap<string, number> {
  let next: Map<string, number> | undefined;
  for (const { key, width } of readings) {
    if (!Number.isFinite(width) || width <= 0) continue;
    const rounded = Math.ceil(width);
    if (rounded <= ((next ?? before).get(key) ?? 0)) continue;
    next ??= new Map(before);
    next.set(key, rounded);
  }
  return next ?? before;
}

export const columnAlign = (column: Pick<TableColumn<unknown>, 'align' | 'numeric'>) => column.align ?? (column.numeric ? 'end' : 'start');

export function cellContent<T>(column: TableColumn<T>, row: T, value = column.value(row)): ReactNode {
  if (isMissing(value) && !column.renderMissing) return '—';
  return column.render ? column.render(row) : isMissing(value) ? '—' : String(value);
}

export function hiddenText<T>(column: TableColumn<T>, row: T, key: string, values: LiveValues<SortValue> | null): { value: string; detail?: string } {
  if (column.paint && values) {
    const cellKey = liveKey(key, column.key), value = values.get(cellKey);
    if (value == null || isMissing(value)) {
      const reason = values.missing(cellKey);
      const shown = column.missingText?.(cellKey, reason) ?? '—';
      return reason && reason !== shown ? { value: shown, detail: reason } : { value: shown };
    }
    return { value: column.paint(value, cellKey) };
  }
  return { value: cellText(column, row) };
}

export function withHidden<T>(content: TooltipContent | null, row: T, hidden: readonly TableColumn<T>[], shown: readonly TableColumn<T>[],
  key: string, viewportHeight: number, values: LiveValues<SortValue> | null = null): TooltipContent | null {
  if (!hidden.length) return content;
  const rows: TooltipRow[] = hidden.map((column, index) => {
    const cell = hiddenText(column, row, key, values);
    return { label: column.label, value: cell.value, ...(cell.detail ? { detail: cell.detail, tone: 'muted' as const } : {}), divider: index === 0 && !!content?.rows.length };
  });
  const first = shown[0];
  const combined: TooltipContent = content
    ? { ...content, rows: [...content.rows, ...rows], spoken: content.spoken && [content.spoken, ...rows.map(item => `${item.label} ${item.value}`)].join('; ') }
    : { title: first ? cellText(first, row) : '', rows };
  return tooltipHeight(combined) > viewportHeight - 16 ? { ...combined, scrollable: true, scrollId: combined.scrollId ?? key } : combined;
}
