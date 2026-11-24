import type { LaneBinding, LaneFamily } from '../laneRuntime.ts';
import { deriveFlowAudit, deriveFlowMirror, deriveFlowNav, findFlowTooltip, type FlowModel } from './model.ts';
import { drawFlow, hitFlow, layoutFlow, type FlowLayout } from './painter.ts';

export interface FamilyFlowBinding<TOptions = unknown> extends LaneBinding<TOptions> {
  readonly family: 'flow';
  readonly width: number;
  readonly height: number;
  readonly pauseFlush?: number;
}

const inspectedItem = (inspect: string | null): string | null => (inspect?.startsWith('node:') || inspect?.startsWith('ribbon:') ? inspect : null);

export const flowLane: LaneFamily<FamilyFlowBinding, FlowModel, FlowLayout> = {
  family: 'flow',
  boundary: () => Infinity,
  redraw: () => Infinity,
  status: model => (model.refused !== undefined ? { refused: model.refused } : 'scene'),
  view: [],
  layout: (model, _binding, surface) => layoutFlow(model, surface.width, surface.height),
  paint(ctx, frame) {
    drawFlow(ctx, {
      model: frame.model,
      layout: frame.layout,
      theme: frame.theme,
      inspected: inspectedItem(frame.inspect),
      focused: frame.binding.focusedKey ?? null,
      artCache: frame.art,
    });
    return false;
  },
  hit: (frame, point) => hitFlow(frame.layout, point[0], point[1] + frame.binding.scrollTop),
  tooltip(frame, key, focus) {
    const item = focus ? key : inspectedItem(key);
    const payload = item ? findFlowTooltip(frame.model, item) : null;
    return payload && item ? { rowKey: item, payload } : null;
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
      nav: deriveFlowNav(model),
      extent: { width: layout.width, height: layout.height },
    };
  },
  mirror: frame => deriveFlowMirror(frame.model),
  audit: frame => deriveFlowAudit(frame.model),
  visibleArt: () => [],
};
