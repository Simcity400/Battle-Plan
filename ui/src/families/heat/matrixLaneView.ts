import type { LaneView } from '../../draw/lane.ts';
import { createLaneView, type LaneBinding, type LaneFamily, type LaneRuntimeOptions } from '../laneRuntime.ts';
import { hitHeatMatrix, layoutHeatMatrix, type HeatMatrixLayout } from './matrixLayout.ts';
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
  ctx.font = font;
  let name = 0;
  let trailing = 0;
  const sampleLimit = Math.min(rows.length, 60);
  for (let i = 0; i < sampleLimit; i++) {
    const row = rows[i]!;
    name = Math.max(name, ctx.measureText(row.label).width);
    if (row.trailing) trailing = Math.max(trailing, ctx.measureText(row.trailing).width);
  }
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

const heatMatrixLane: LaneFamily<FamilyHeatMatrixBinding, HeatMatrixModel, HeatMatrixLayout> = {
  family: 'heat-matrix',
  boundary: () => Infinity,
  redraw: () => Infinity,
  status: model => (model.refused !== undefined ? { refused: model.refused } : 'scene'),
  view: binding => [binding.scrollLeft, binding.scrollTop],
  layout(model, binding, surface, theme, measure) {
    const widest = widestOf(model, `500 ${theme.fontSize.small}px ${theme.font}`, measure);
    return layoutHeatMatrix({
      width: surface.width,
      height: surface.height,
      scrollLeft: binding.scrollLeft,
      scrollTop: binding.scrollTop,
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
      layout: frame.layout,
      theme: frame.theme,
      scrollLeft: frame.binding.scrollLeft,
      scrollTop: frame.binding.scrollTop,
      hoveredCell: hoveredCellOf(frame.inspect),
      focusedCell: parseMatrixCellKey(frame.binding.focusedKey ?? null),
      artProvider: frame.artProvider,
      identityIndex: frame.index,
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
    return payload ? { hour: frame.model.hour, rowKey: matrixCellKey(cell.row, cell.column), payload } : null;
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
        rows: model.rows,
        columns: model.columns,
      },
    };
  },
  mirror: ({ model, layout }) => deriveMatrixMirror(model, layout.firstRow, layout.endRow, layout.firstCol, layout.endCol),
  audit: ({ model, layout }) => deriveMatrixAudit(model, { firstRow: layout.firstRow, endRow: layout.endRow, firstCol: layout.firstCol, endCol: layout.endCol }),
  visibleArt: frame => collectVisibleMatrixArt(frame.model, frame.layout, frame.index),
};

export function familyHeatMatrixLaneView(wake: (widget: number) => void, options?: LaneRuntimeOptions): LaneView {
  return createLaneView(heatMatrixLane, wake, options);
}
