import type { ReactNode } from 'react';
import { isMissing } from './logic.ts';
import type { SortValue } from './logic';

/** One model for text, numbers, portraits and composed cells. */
export interface TableColumn<T> {
  key: string;
  header: ReactNode;
  /** Accessible header name and the settings' Sort by text, even if header is empty. */
  label: string;
  value: (row: T) => SortValue;
  render?: (row: T) => ReactNode;
  renderMissing?: boolean;
  align?: 'start' | 'end';
  numeric?: boolean;
  sortable?: boolean;
  /** Positive maximum in the value's units, or fit this filtered table. */
  bar?: 'visible' | number | bigint;
  width?: string;
}

export const columnAlign = (column: Pick<TableColumn<unknown>, 'align' | 'numeric'>) => column.align ?? (column.numeric ? 'end' : 'start');

/** Missing sort/bar values also gate custom content unless explicitly opted in. */
export function cellContent<T>(column: TableColumn<T>, row: T, value = column.value(row)): ReactNode {
  if (isMissing(value) && !column.renderMissing) return '—';
  return column.render ? column.render(row) : isMissing(value) ? '—' : String(value);
}
