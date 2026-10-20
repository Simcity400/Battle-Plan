import { mix } from '../../shell/appearance/color.ts';
import { FLAG, NAME_PAD } from '../race/draw.ts';
import type { CountryFlagLook } from '../../components/CountryFlag';

/** One row or column of a matrix: an opaque ID, its full label and, for a country, its flag. */
export interface MatrixAxisItem { id: string; label: string; countryFlag?: CountryFlagLook }
/** A drawn cell: a fill, the kit's unknown hatch, or nothing (an empty cell such as no value). */
export type MatrixCell = { color: string } | { unknown: true } | null;

/** The least cell edge: a flag and its margins fit the column header above it. */
export const MATRIX_MIN_CELL = FLAG.width + 6;
/** Small matrices grow their cells up to this edge rather than leave the card empty. */
export const MATRIX_MAX_CELL = 40;
/** The column header: the flags above their columns. */
export const MATRIX_HEADER = FLAG.height + 12;
/** Below this card width row names give way to flags, with names in the tooltip. */
export const MATRIX_NAMES_WIDTH = 420;

/** Cells share the room after the row gutter, square, never smaller than a flag needs. */
export function matrixCell(width: number, height: number, gutter: number, rows: number, columns: number): number {
  const across = (width - gutter) / Math.max(1, columns), down = (height - MATRIX_HEADER) / Math.max(1, rows);
  const fit = Math.min(across, down);
  return Math.max(MATRIX_MIN_CELL, Math.min(MATRIX_MAX_CELL, Number.isFinite(fit) ? Math.floor(fit) : MATRIX_MIN_CELL));
}

/** The row gutter: a flag and the widest name, at most 45% of the card; flags alone when narrow. */
export function matrixGutter(width: number, widestName: number, names: boolean, flags: boolean): number {
  const flag = flags ? FLAG.x + FLAG.width : FLAG.x;
  if (!names) return flag + NAME_PAD;
  return Math.ceil(Math.min(Math.max(flag + NAME_PAD, width * 0.45), flag + NAME_PAD + widestName + NAME_PAD));
}

/** The items of one axis that a viewport shows, with one item of margin either side. */
export function matrixWindow(count: number, scroll: number, size: number, cell: number) {
  const first = Math.max(0, Math.floor(scroll / cell) - 1);
  const end = Math.min(count, Math.ceil((scroll + Math.max(0, size)) / cell) + 1);
  return { first, end };
}

/** The cell under a point in the plot (after the gutter and header), or null outside it. */
export function matrixCellAt(x: number, y: number, layout: { gutter: number; header: number; cell: number; scrollLeft: number; scrollTop: number },
  rows: number, columns: number): { row: number; column: number } | null {
  if (x < layout.gutter || y < layout.header) return null;
  const column = Math.floor((x - layout.gutter + layout.scrollLeft) / layout.cell);
  const row = Math.floor((y - layout.header + layout.scrollTop) / layout.cell);
  return row >= 0 && row < rows && column >= 0 && column < columns ? { row, column } : null;
}

/** Arrow keys step a cell, Home/End go to the row's first/last column, Page keys a viewport. */
export function matrixKey(key: string, at: { row: number; column: number }, rows: number, columns: number, page: number): { row: number; column: number } | null {
  if (!rows || !columns) return null;
  const clamp = (value: number, count: number) => Math.max(0, Math.min(count - 1, value));
  switch (key) {
    case 'ArrowDown': return { row: clamp(at.row + 1, rows), column: at.column };
    case 'ArrowUp': return { row: clamp(at.row - 1, rows), column: at.column };
    case 'ArrowRight': return { row: at.row, column: clamp(at.column + 1, columns) };
    case 'ArrowLeft': return { row: at.row, column: clamp(at.column - 1, columns) };
    case 'PageDown': return { row: clamp(at.row + Math.max(1, page), rows), column: at.column };
    case 'PageUp': return { row: clamp(at.row - Math.max(1, page), rows), column: at.column };
    case 'Home': return { row: at.row, column: 0 };
    case 'End': return { row: at.row, column: columns - 1 };
    default: return null;
  }
}

/** The scroll offset that brings an item fully into a viewport of `size`. */
export function revealItem(index: number, cell: number, scroll: number, size: number): number {
  const start = index * cell, end = start + cell;
  if (start < scroll) return start;
  if (end > scroll + size) return Math.max(0, end - size);
  return scroll;
}

/** A signed value's colour: theme grey at zero, fading to `negative` at `low` (below zero) or
 * `positive` at `high` (above). The domain is the caller's; a value beyond it takes the full
 * colour, and a side with no extent stays grey. */
export function signedColor(value: number, low: number, high: number, grey: string, negative: string, positive: string): string {
  if (!Number.isFinite(value) || value === 0) return grey;
  const extent = value < 0 ? Math.abs(Math.min(0, low)) : Math.max(0, high);
  if (!(extent > 0)) return grey;
  return mix(grey, value < 0 ? negative : positive, Math.min(1, Math.abs(value) / extent));
}
