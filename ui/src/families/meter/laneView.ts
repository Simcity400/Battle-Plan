import type { LaneBinding, LaneFamily } from '../laneRuntime.ts';
import {
  deriveMeterAudit,
  deriveMeterMirror,
  deriveMeterNav,
  findMeterTooltip,
  nextMeterChange,
  type MeterModel,
} from './model.ts';
import { drawMeterModel, hitMeterModel, layoutMeterModel, type MeterLayout } from './painter.ts';

export interface FamilyMeterBinding<TOptions = unknown> extends LaneBinding<TOptions> {
  readonly family: 'meters';
  readonly width: number;
  readonly height: number;
  readonly pauseFlush?: number;
}

const inspectedRow = (inspect: string | null): string | null => (inspect?.startsWith('row:') ? inspect.slice(4) : null);

export const meterLane: LaneFamily<FamilyMeterBinding, MeterModel, MeterLayout> = {
  family: 'meters',
  boundary: nextMeterChange,
  redraw: () => Infinity,
  status: model => (model.refused !== undefined ? { refused: model.refused } : 'scene'),
  view: [],
  layout: (model, _binding, surface, theme, measure) => layoutMeterModel(measure, model, surface.width, theme),
  paint(ctx, frame) {
    drawMeterModel(ctx, {
      layout: frame.layout,
      width: frame.surface.width,
      height: frame.surface.height,
      scrollLeft: 0,
      scrollTop: frame.binding.scrollTop,
      theme: frame.theme,
      inspectedKey: inspectedRow(frame.inspect),
      selectedKey: frame.binding.selectedKey ?? null,
      focusedKey: frame.binding.focusedKey ?? null,
    });
    return false;
  },
  hit: (frame, point) => hitMeterModel(frame.layout, point[0], point[1], 0, frame.binding.scrollTop),
  tooltip(frame, key, focus) {
    const rowKey = focus ? key : inspectedRow(key);
    const payload = rowKey ? findMeterTooltip(frame.model, rowKey) : null;
    return payload && rowKey ? { rowKey, payload } : null;
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
        layout: model.layout,
      },
      nav: deriveMeterNav(model, layout.itemBands),
      extent: { width: layout.contentWidth, height: layout.contentHeight },
    };
  },
  mirror: frame => deriveMeterMirror(frame.model),
  audit: frame => deriveMeterAudit(frame.model),
  visibleArt: () => [],
};
