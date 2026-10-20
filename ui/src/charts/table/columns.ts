import type { ReactNode } from 'react';
import { usNumber } from '../numberFormat.ts';
import { tooltipHeight } from '../tooltip.ts';
import type { TooltipContent } from '../tooltip';
import { isMissing } from './logic.ts';
import type { SortValue, SortDirection } from './logic';

/** One model for text, numbers, portraits and composed cells. */
export interface TableColumn<T> {
  key: string;
  header: ReactNode;
  /** Accessible header name and the settings' Sort by text, even if header is empty. */
  label: string;
  /** Header hint, including why a requested measure is unavailable. */
  hint?: string;
  value: (row: T) => SortValue;
  render?: (row: T) => ReactNode;
  renderMissing?: boolean;
  align?: 'start' | 'end';
  numeric?: boolean;
  sortable?: boolean;
  /** A fixed choice keeps this direction on every header click. */
  direction?: SortDirection;
  /** Positive maximum in the value's units, or fit this filtered table. */
  bar?: 'visible' | number | bigint;
  /** CSS grid track; max-content shares the widest mounted header/cell across rows. */
  width?: string;
  /** As the table narrows, columns with a priority hide, the largest number first, and
   * their values move into the row tooltip. Columns without one always show. */
  priority?: number;
  /** A hidden column's value in the row tooltip, when its cell is not plain text. */
  text?: (row: T) => string | null;
  /** The widget's row tooltip already shows this value, so hiding the column adds nothing. */
  inTooltip?: boolean;
}

/** Hiding order: the largest priority first; equal priorities hide the later column first. */
export function hideOrder<T>(columns: readonly TableColumn<T>[]): string[] {
  return columns.map((column, index) => ({ column, index }))
    .filter(({ column }) => column.priority != null && Number.isFinite(column.priority))
    .sort((a, b) => b.column.priority! - a.column.priority! || b.index - a.index)
    .map(({ column }) => column.key);
}

/** The text a hidden column contributes to the row tooltip; missing reads as the cell's dash. */
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

/** Content-sized tracks grow as virtual rows appear; names can keep zero-minimum
 * fractional tracks without making the whole table as wide as their full text. */
export function tableGrid<T>(columns: readonly TableColumn<T>[], rank: boolean, widths: ReadonlyMap<string, number>): string {
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

/** Missing sort/bar values also gate custom content unless explicitly opted in. */
export function cellContent<T>(column: TableColumn<T>, row: T, value = column.value(row)): ReactNode {
  if (isMissing(value) && !column.renderMissing) return '—';
  return column.render ? column.render(row) : isMissing(value) ? '—' : String(value);
}

/** Hidden columns' values follow the widget's own rows; a table without a tooltip
 * titles the row by its first shown column. */
export function withHidden<T>(content: TooltipContent | null, row: T, hidden: readonly TableColumn<T>[], shown: readonly TableColumn<T>[],
  key: string, viewportHeight: number): TooltipContent | null {
  if (!hidden.length) return content;
  const rows = hidden.map((column, index) => ({ label: column.label, value: cellText(column, row), divider: index === 0 && !!content?.rows.length }));
  const first = shown[0];
  const combined: TooltipContent = content
    ? { ...content, rows: [...content.rows, ...rows], spoken: content.spoken && [content.spoken, ...rows.map(item => `${item.label} ${item.value}`)].join('; ') }
    : { title: first ? cellText(first, row) : '', rows };
  // A widget fits its tooltip to the window before the hidden values join it; when they
  // make it taller than the window, the shared scrolling layer keeps every row reachable.
  return tooltipHeight(combined) > viewportHeight - 16 ? { ...combined, scrollable: true, scrollId: combined.scrollId ?? key } : combined;
}
