import type { ReactNode } from 'react';
import { isMissing } from './logic.ts';
import type { SortValue } from './logic';

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
  /** Positive maximum in the value's units, or fit this filtered table. */
  bar?: 'visible' | number | bigint;
  /** CSS grid track; max-content shares the widest mounted header/cell across rows. */
  width?: string;
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
