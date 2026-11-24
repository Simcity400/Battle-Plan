import type { LaneView } from '../../draw/lane.ts';
import type { FamilyNavTuple } from '../host/keyboard.ts';
import { createLaneView, type LaneBinding, type LaneFamily, type LaneRuntimeOptions } from '../laneRuntime.ts';
import { hitHeatLanes, layoutHeatLanes, type HeatLanesLayout } from './lanesLayout.ts';
import {
  deriveLanesAudit,
  deriveLanesMirror,
  deriveLanesTooltip,
  laneIdentities,
  laneInspectKey,
  parseLaneInspectKey,
  type HeatLanesModel,
} from './lanesModel.ts';
import { collectVisibleLaneArt, drawHeatLanes } from './lanesPainter.ts';

export interface FamilyHeatLanesBinding<TOptions = unknown> extends LaneBinding<TOptions> {
  readonly family: 'heat-lanes';
  readonly width: number;
  readonly height: number;
  readonly pauseFlush?: number;
}

const inspectedLane = (inspect: string | null) => parseLaneInspectKey(inspect?.startsWith('lane:') ? inspect.slice(5) : null);

const heatLanesLane: LaneFamily<FamilyHeatLanesBinding, HeatLanesModel, HeatLanesLayout> = {
  family: 'heat-lanes',
  boundary: () => Infinity,
  redraw: () => Infinity,
  status: model => (model.refused !== undefined ? { refused: model.refused } : 'scene'),
  view: binding => [binding.scrollTop],
  layout: (model, binding, surface) => layoutHeatLanes({
    width: surface.width,
    height: surface.height,
    scrollTop: binding.scrollTop,
    firstHour: model.firstHour,
    lastHour: model.lastHour,
    displayedHour: model.hour,
    rows: model.rows,
    legend: model.legend,
    requestedReadoutWidth: model.readoutWidth,
    requestedRowHeight: model.rowHeight,
  }),
  paint(ctx, frame) {
    const hovered = inspectedLane(frame.inspect);
    drawHeatLanes(ctx, {
      model: frame.model,
      layout: frame.layout,
      theme: frame.theme,
      scrollTop: frame.binding.scrollTop,
      hoveredRow: hovered?.rowIndex ?? null,
      hoveredHour: hovered?.hour ?? null,
      selectedRowId: frame.binding.selectedKey ?? null,
      focusedRowId: frame.binding.focusedKey ?? null,
      artProvider: frame.artProvider,
      identityIndex: frame.index,
    });
    return false;
  },
  hit(frame, point) {
    const hit = hitHeatLanes(point[0], point[1], frame.binding.scrollTop, frame.layout, frame.model.rows.length);
    if (!hit) return null;
    return `lane:${laneInspectKey(hit.rowIndex, hit.kind === 'lane' ? hit.hour : null)}`;
  },
  tooltip(frame, key, focus) {
    const { model } = frame;
    if (focus) {
      const row = model.rows.findIndex(item => item.id === key);
      const payload = row >= 0 ? deriveLanesTooltip(model, row, model.hour) : null;
      return payload ? { hour: model.hour, rowKey: key, payload } : null;
    }
    const hovered = inspectedLane(key);
    if (!hovered) return null;
    const payload = deriveLanesTooltip(model, hovered.rowIndex, hovered.hour);
    return payload ? { hour: hovered.hour ?? model.hour, rowKey: laneInspectKey(hovered.rowIndex, hovered.hour), payload } : null;
  },
  structure({ model, layout }) {
    return {
      meta: {
        empty: model.empty ?? null,
        emptyDetail: model.emptyDetail ?? null,
        emptyKind: model.emptyKind ?? 'empty',
        notes: model.notes ?? [],
        warnings: model.warnings ?? [],
      },
      extent: { totalHeight: layout.totalHeight, rowStride: layout.rowStride, plotTop: layout.plotTop, plotHeight: layout.plotHeight, rows: laneIdentities(model.rows) },
      nav: model.rows.map((row, index): FamilyNavTuple => [row.id, row.label, index, -1, '', row.meta ?? null, index * layout.rowStride, layout.rowHeight]),
    };
  },
  mirror: frame => deriveLanesMirror(frame.model, frame.layout.firstRow, frame.layout.endRow),
  audit: frame => deriveLanesAudit(frame.model, { firstRow: frame.layout.firstRow, endRow: frame.layout.endRow }),
  visibleArt: frame => collectVisibleLaneArt(frame.model, frame.layout, frame.index),
};

export function familyHeatLanesLaneView(wake: (widget: number) => void, options?: LaneRuntimeOptions): LaneView {
  return createLaneView(heatLanesLane, wake, options);
}
