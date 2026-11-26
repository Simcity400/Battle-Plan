import { FLAG_SLOT_HEIGHT, FLAG_SLOT_WIDTH } from '../../draw/kit.ts';
import type { MatrixAxisEntity, MatrixLegend } from './matrixModel.ts';
import { matrixGroupSpans, type MatrixGroupSpan } from './matrixModel.ts';

export const MATRIX_MIN_CELL = 26;
export const MATRIX_MAX_CELL = 44;
export const MATRIX_NAME_MAX_WIDTH = 168;
export const MATRIX_COMPACT_WIDTH = 420;
export const MATRIX_LEGEND_HEIGHT = 24;
export const MATRIX_GROUP_BAND_HEIGHT = 18;
export const MATRIX_COL_HEADER_HEIGHT = FLAG_SLOT_HEIGHT + 16;
export const MATRIX_PAD = 8;
export const MATRIX_WIDE_MIN_ROW = 16;
export const MATRIX_WIDE_MAX_ROW = 24;
export const MATRIX_WIDE_MIN_CELL = 34;
export const MATRIX_WIDE_COMPACT_WIDTH = 360;
export const MATRIX_COLUMN_NAMES_MIN_HEIGHT = 180;
export const MATRIX_WIDE_MAX_CELL = 64;
export const MATRIX_COLUMN_NAME_HEIGHT = 50;
export const MATRIX_COLUMN_NAME_ANGLE = (40 * Math.PI) / 180;
export const MATRIX_MARGIN_WIDTH = 112;
export const MATRIX_MARGIN_MIN_WIDTH = 520;

export interface HeatMatrixLayout {
  readonly width: number;
  readonly height: number;
  readonly legendHeight: number;
  readonly colGroupHeight: number;
  readonly headerHeight: number;
  readonly rowGroupWidth: number;
  readonly gutterWidth: number;
  readonly showNames: boolean;
  readonly showFlags: boolean;
  readonly cellSize: number;
  readonly cellWidth: number;
  readonly cellHeight: number;
  readonly wide: boolean;
  readonly columnNames: boolean;
  readonly marginWidth: number;
  readonly gridLeft: number;
  readonly gridTop: number;
  readonly trailingWidth: number;
  readonly totalWidth: number;
  readonly totalHeight: number;
  readonly firstRow: number;
  readonly endRow: number;
  readonly firstCol: number;
  readonly endCol: number;
  readonly rowGroups: readonly MatrixGroupSpan[];
  readonly colGroups: readonly MatrixGroupSpan[];
}

export interface LayoutHeatMatrixInput {
  readonly width: number;
  readonly height: number;
  readonly scrollLeft: number;
  readonly scrollTop: number;
  readonly rows: readonly MatrixAxisEntity[];
  readonly columns: readonly MatrixAxisEntity[];
  readonly legend?: MatrixLegend;
  readonly widestRowName?: number;
  readonly shape?: 'square' | 'wide';
  readonly margin?: boolean;
  readonly widestTrailing?: number;
}

export const MATRIX_TRAILING_GAP = 16;

interface AxisFacts {
  readonly groups: readonly MatrixGroupSpan[];
  readonly flags: boolean;
}

const heldAxes = new WeakMap<readonly MatrixAxisEntity[], AxisFacts>();

function axisFacts(items: readonly MatrixAxisEntity[]): AxisFacts {
  let facts = heldAxes.get(items);
  if (!facts) {
    facts = { groups: matrixGroupSpans(items), flags: items.some(item => item.flag !== undefined || item.tag !== undefined) };
    heldAxes.set(items, facts);
  }
  return facts;
}

export function layoutHeatMatrix(input: LayoutHeatMatrixInput): HeatMatrixLayout {
  const width = Math.max(1, Math.floor(input.width));
  const height = Math.max(1, Math.floor(input.height));
  const hasLegend = Boolean(
    input.legend && ((input.legend.entries?.length ?? 0) > 0 || input.legend.gradient || input.legend.rightNote),
  );
  const legendHeight = hasLegend ? MATRIX_LEGEND_HEIGHT : 0;
  const rowAxis = axisFacts(input.rows);
  const colAxis = axisFacts(input.columns);
  const rowGroups = rowAxis.groups;
  const colGroups = colAxis.groups;
  const colGroupHeight = colGroups.length > 0 ? MATRIX_GROUP_BAND_HEIGHT : 0;
  const rowGroupWidth = rowGroups.length > 0 ? 4 : 0;
  const showFlags = rowAxis.flags || colAxis.flags;
  const wide = input.shape === 'wide';
  const showNames = width >= (wide ? MATRIX_WIDE_COMPACT_WIDTH : MATRIX_COMPACT_WIDTH);

  const flagPart = showFlags ? FLAG_SLOT_WIDTH + 8 : 0;
  const rawNameWidth = input.widestRowName !== undefined
    ? Math.ceil(input.widestRowName)
    : MATRIX_NAME_MAX_WIDTH - flagPart - 16;
  const gutterWidth = showNames
    ? Math.min(MATRIX_NAME_MAX_WIDTH, Math.max(flagPart + 48, rowGroupWidth + MATRIX_PAD + flagPart + rawNameWidth + MATRIX_PAD))
    : rowGroupWidth + MATRIX_PAD + (showFlags ? FLAG_SLOT_WIDTH : 28) + MATRIX_PAD;

  const columnNames = wide && showNames && height >= MATRIX_COLUMN_NAMES_MIN_HEIGHT;
  const headerHeight = colGroupHeight + MATRIX_COL_HEADER_HEIGHT + (columnNames ? MATRIX_COLUMN_NAME_HEIGHT : 0);
  const gridLeft = gutterWidth;
  const gridTop = legendHeight + headerHeight;
  const marginWidth = input.margin && width >= MATRIX_MARGIN_MIN_WIDTH ? MATRIX_MARGIN_WIDTH : 0;

  const trailingWidth = input.widestTrailing ? Math.ceil(input.widestTrailing) + MATRIX_TRAILING_GAP + MATRIX_PAD : 0;
  const availW = Math.max(MATRIX_MIN_CELL, width - gridLeft - marginWidth - trailingWidth);
  const availH = Math.max(MATRIX_MIN_CELL, height - gridTop);
  const fitW = input.columns.length > 0 ? Math.floor(availW / input.columns.length) : MATRIX_MAX_CELL;
  const fitH = input.rows.length > 0 ? Math.floor(availH / input.rows.length) : MATRIX_MAX_CELL;
  const square = Math.max(MATRIX_MIN_CELL, Math.min(MATRIX_MAX_CELL, Math.min(fitW, fitH)));
  const cellWidth = wide ? Math.max(MATRIX_WIDE_MIN_CELL, Math.min(MATRIX_WIDE_MAX_CELL, fitW)) : square;
  const cellHeight = wide ? Math.max(MATRIX_WIDE_MIN_ROW, Math.min(MATRIX_WIDE_MAX_ROW, fitH)) : square;
  const cellSize = cellHeight;

  const totalWidth = Math.max(width, gridLeft + input.columns.length * cellWidth + trailingWidth + marginWidth);
  const totalHeight = Math.max(height, gridTop + input.rows.length * cellHeight);

  const { firstRow, endRow, firstCol, endCol } = matrixWindow(
    { width, height, gridLeft, gridTop, marginWidth, cellWidth, cellHeight },
    input.scrollLeft,
    input.scrollTop,
    input.rows.length,
    input.columns.length,
  );

  return {
    width,
    height,
    legendHeight,
    colGroupHeight,
    headerHeight,
    rowGroupWidth,
    gutterWidth,
    showNames,
    showFlags,
    cellSize,
    cellWidth,
    cellHeight,
    wide,
    columnNames,
    marginWidth,
    gridLeft,
    gridTop,
    trailingWidth,
    totalWidth,
    totalHeight,
    firstRow,
    endRow,
    firstCol,
    endCol,
    rowGroups,
    colGroups,
  };
}

export interface MatrixGeometry {
  readonly width: number;
  readonly height: number;
  readonly gridLeft: number;
  readonly gridTop: number;
  readonly marginWidth: number;
  readonly cellWidth: number;
  readonly cellHeight: number;
}

export interface MatrixWindow {
  readonly firstRow: number;
  readonly endRow: number;
  readonly firstCol: number;
  readonly endCol: number;
}

export function matrixWindow(geometry: MatrixGeometry, scrollLeftRaw: number, scrollTopRaw: number, rows: number, columns: number): MatrixWindow {
  const scrollLeft = Math.max(0, scrollLeftRaw);
  const scrollTop = Math.max(0, scrollTopRaw);
  const visibleW = Math.max(0, geometry.width - geometry.gridLeft - geometry.marginWidth);
  const visibleH = Math.max(0, geometry.height - geometry.gridTop);
  return {
    firstCol: Math.max(0, Math.floor(scrollLeft / geometry.cellWidth) - 1),
    endCol: Math.min(columns, Math.ceil((scrollLeft + visibleW) / geometry.cellWidth) + 1),
    firstRow: Math.max(0, Math.floor(scrollTop / geometry.cellHeight) - 1),
    endRow: Math.min(rows, Math.ceil((scrollTop + visibleH) / geometry.cellHeight) + 1),
  };
}

export type MatrixHit =
  | { readonly kind: 'cell'; readonly row: number; readonly column: number }
  | { readonly kind: 'row'; readonly row: number }
  | { readonly kind: 'column'; readonly column: number }
  | { readonly kind: 'margin'; readonly row: number }
  | null;

export function hitHeatMatrix(
  x: number,
  y: number,
  scrollLeft: number,
  scrollTop: number,
  layout: HeatMatrixLayout,
  rowCount: number,
  colCount: number,
): MatrixHit {
  if (x < 0 || y < layout.legendHeight || x > layout.width || y > layout.height) return null;
  const marginLeft = layout.width - layout.marginWidth;
  if (layout.marginWidth > 0 && x >= marginLeft && y >= layout.gridTop) {
    const row = Math.floor((y - layout.gridTop + scrollTop) / layout.cellHeight);
    return row >= 0 && row < rowCount ? { kind: 'margin', row } : null;
  }
  if (x >= layout.gridLeft && y >= layout.gridTop) {
    const column = Math.floor((x - layout.gridLeft + scrollLeft) / layout.cellWidth);
    const row = Math.floor((y - layout.gridTop + scrollTop) / layout.cellHeight);
    if (row >= 0 && row < rowCount && column >= 0 && column < colCount) {
      return { kind: 'cell', row, column };
    }
    return null;
  }
  if (x < layout.gridLeft && y >= layout.gridTop) {
    const row = Math.floor((y - layout.gridTop + scrollTop) / layout.cellHeight);
    return row >= 0 && row < rowCount ? { kind: 'row', row } : null;
  }
  if (x >= layout.gridLeft && x < marginLeft && y >= layout.legendHeight && y < layout.gridTop) {
    const column = Math.floor((x - layout.gridLeft + scrollLeft) / layout.cellWidth);
    return column >= 0 && column < colCount ? { kind: 'column', column } : null;
  }
  return null;
}
