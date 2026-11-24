import { fitLabelWidth } from '../../charts/labels.ts';
import { measureCanvasText } from '../../charts/textWidth.ts';
import type { LaneBinding, LaneFamily } from '../laneRuntime.ts';
import { hitHeatMatrix, layoutHeatMatrix, matrixWindow, type HeatMatrixLayout } from './matrixLayout.ts';
import {
  deriveMatrixAudit,
  deriveMatrixMirror,
  deriveMatrixTooltip,
  matrixCellKey,
  parseMatrixCellKey,
  type HeatMatrixModel,
} from './matrixModel.ts';
import { collectVisibleMatrixArt, drawHeatMatrix } from './matrixPainter.ts';

export interface FamilyHeatMatrixBinding<TOptions = unknown> extends LaneBinding<TOptions> {
  readonly family: 'heat-matrix';
  readonly width: number;
  readonly height: number;
  readonly scrollLeft: number;
  readonly pauseFlush?: number;
}

export function widestNames(
  ctx: { font: string; measureText(text: string): { width: number } },
  font: string,
  rows: readonly { readonly label: string; readonly trailing?: string }[],
): { readonly name: number; readonly trailing: number } {
  const sample = rows.slice(0, 60);
  const measure = (text: string) => measureCanvasText(ctx as Parameters<typeof measureCanvasText>[0], text, font);
  const name = fitLabelWidth(sample, row => measure(row.label), 0, Number.MAX_SAFE_INTEGER);
  const trailing = fitLabelWidth(sample, row => (row.trailing ? measure(row.trailing) : 0), 0, Number.MAX_SAFE_INTEGER);
  return { name, trailing };
}

const widths = new WeakMap<HeatMatrixModel, { readonly font: string; readonly name: number; readonly trailing: number }>();

function widestOf(model: HeatMatrixModel, font: string, measure: OffscreenCanvasRenderingContext2D): { readonly name: number; readonly trailing: number } {
  const held = widths.get(model);
  if (held && held.font === font) return held;
  const widest = { font, ...widestNames(measure, font, model.rows) };
  widths.set(model, widest);
  return widest;
}

const hoveredCellOf = (inspect: string | null) => parseMatrixCellKey(inspect?.startsWith('cell:') ? inspect.slice(5) : null);

const windows = new WeakMap<HeatMatrixLayout, { readonly scrollLeft: number; readonly scrollTop: number; readonly layout: HeatMatrixLayout }>();

function shown(frame: { readonly model: HeatMatrixModel; readonly layout: HeatMatrixLayout; readonly binding: { readonly scrollLeft: number; readonly scrollTop: number } }): HeatMatrixLayout {
  const { scrollLeft, scrollTop } = frame.binding;
  const held = windows.get(frame.layout);
  if (held && held.scrollLeft === scrollLeft && held.scrollTop === scrollTop) return held.layout;
  const layout = { ...frame.layout, ...matrixWindow(frame.layout, scrollLeft, scrollTop, frame.model.rows.length, frame.model.columns.length) };
  windows.set(frame.layout, { scrollLeft, scrollTop, layout });
  return layout;
}

export const heatMatrixLane: LaneFamily<FamilyHeatMatrixBinding, HeatMatrixModel, HeatMatrixLayout> = {
  family: 'heat-matrix',
  boundary: () => Infinity,
  redraw: () => Infinity,
  status: model => (model.refused !== undefined ? { refused: model.refused } : 'scene'),
  view: [],
  layout(model, _binding, surface, theme, measure) {
    const widest = widestOf(model, `500 ${theme.fontSize.small}px ${theme.font}`, measure);
    return layoutHeatMatrix({
      width: surface.width,
      height: surface.height,
      scrollLeft: 0,
      scrollTop: 0,
      rows: model.rows,
      columns: model.columns,
      legend: model.legend,
      widestRowName: widest.name > 0 ? widest.name : undefined,
      widestTrailing: widest.trailing > 0 ? widest.trailing : undefined,
      shape: model.shape,
      margin: model.margin !== undefined,
    });
  },
  paint(ctx, frame) {
    drawHeatMatrix(ctx, {
      model: frame.model,
      layout: shown(frame),
      theme: frame.theme,
      scrollLeft: frame.binding.scrollLeft,
      scrollTop: frame.binding.scrollTop,
      hoveredCell: hoveredCellOf(frame.inspect),
      focusedCell: parseMatrixCellKey(frame.binding.focusedKey ?? null),
      artCache: frame.art,
    });
    return false;
  },
  hit(frame, point) {
    const columns = frame.model.columns.length;
    const hit = hitHeatMatrix(point[0], point[1], frame.binding.scrollLeft, frame.binding.scrollTop, frame.layout, frame.model.rows.length, columns);
    if (!hit) return null;
    if (hit.kind === 'cell') return `cell:${hit.row}:${hit.column}`;
    if (hit.kind === 'margin') return `cell:${hit.row}:${columns}`;
    return null;
  },
  tooltip(frame, key, focus) {
    const cell = focus ? parseMatrixCellKey(key) : hoveredCellOf(key);
    if (!cell) return null;
    const payload = deriveMatrixTooltip(frame.model, cell.row, cell.column);
    return payload ? { rowKey: matrixCellKey(cell.row, cell.column), payload } : null;
  },
  structure({ model, layout }) {
    return {
      meta: {
        empty: model.empty ?? null,
        emptyDetail: model.emptyDetail ?? null,
        emptyKind: model.emptyKind ?? 'empty',
        summaryText: model.summaryText ?? null,
        notes: model.notes ?? [],
        warnings: model.warnings ?? [],
      },
      extent: {
        totalWidth: layout.totalWidth,
        totalHeight: layout.totalHeight,
        cellSize: layout.cellSize,
        cellWidth: layout.cellWidth,
        gridLeft: layout.gridLeft,
        gridTop: layout.gridTop,
        marginWidth: layout.marginWidth,
        margin: model.margin ? model.margin.header : null,
        rows: model.rows,
        columns: model.columns,
      },
    };
  },
  mirror: frame => {
    const window = shown(frame);
    return deriveMatrixMirror(frame.model, window.firstRow, window.endRow, window.firstCol, window.endCol);
  },
  audit: frame => {
    const window = shown(frame);
    return deriveMatrixAudit(frame.model, { firstRow: window.firstRow, endRow: window.endRow, firstCol: window.firstCol, endCol: window.endCol });
  },
  visibleArt: frame => collectVisibleMatrixArt(frame.model, shown(frame)),
};
