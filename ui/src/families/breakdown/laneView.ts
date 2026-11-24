import { canvasMeasureText } from '../../charts/textWidth.ts';
import { collectArtUrls, type ArtUrlInput } from '../art/provider.ts';
import type { MirrorModel } from '../contract.ts';
import type { FamilyNavTuple } from '../host/keyboard.ts';
import type { LaneBinding, LaneFamily } from '../laneRuntime.ts';
import type { BreakdownViewOptions } from './declaration.ts';
import {
  buildBreakdownTooltip,
  breakdownLevel,
  type BreakdownModel,
  type BreakdownPart,
  type ResolvedBreakdownLevel,
} from './model.ts';
import { hitTestBreakdown, layoutBreakdown, type BreakdownLayout } from './layout.ts';
import { drawBreakdown } from './painter.ts';
import { deriveBreakdownAudit, deriveBreakdownNavAndMirror } from './structure.ts';

export interface FamilyBreakdownBinding<TOptions extends BreakdownViewOptions = BreakdownViewOptions> extends LaneBinding<TOptions> {
  readonly family: 'breakdown';
  readonly width: number;
  readonly height: number;
  readonly expanded: Readonly<Record<string, boolean>>;
}

interface BreakdownFrameLayout {
  readonly level: ResolvedBreakdownLevel;
  readonly layout: BreakdownLayout;
  readonly contentHeight: number;
  readonly nav: readonly FamilyNavTuple[];
  readonly mirror: MirrorModel;
}

function walkBreakdownArt(parts: readonly BreakdownPart[], out: ArtUrlInput[]): ArtUrlInput[] {
  for (const part of parts) {
    out.push(part.flagArt);
    if (part.children) walkBreakdownArt(part.children, out);
  }
  return out;
}

const inspectedRow = (inspect: string | null): string | null => (inspect?.startsWith('row:') ? inspect.slice(4) : null);

export const breakdownLane: LaneFamily<FamilyBreakdownBinding, BreakdownModel, BreakdownFrameLayout> = {
  family: 'breakdown',
  boundary: () => Infinity,
  redraw: () => Infinity,
  status: () => 'scene',
  view: ['expanded'],
  layout(model, binding, surface, theme, measure) {
    const level = breakdownLevel(model);
    const layout = layoutBreakdown({
      view: binding.options.view,
      level,
      periodLabel: model.periodLabel,
      comparison: model.comparison,
      width: surface.width,
      height: surface.height,
      theme,
      namesInRing: binding.options.namesInRing,
      userExpanded: binding.expanded,
      measure: canvasMeasureText(measure),
    });
    const { nav, mirror } = deriveBreakdownNavAndMirror(layout, level.total);
    return { level, layout, contentHeight: layout.contentHeight, nav, mirror };
  },
  paint(ctx, frame) {
    drawBreakdown(ctx, {
      layout: frame.layout.layout,
      theme: frame.theme,
      hour: frame.model.hour,
      scrollTop: frame.binding.scrollTop,
      inspectedKey: inspectedRow(frame.inspect),
      selectedKey: frame.binding.selectedKey ?? null,
      focusedKey: frame.binding.focusedKey ?? null,
      artCache: frame.art,
    });
    return false;
  },
  hit: (frame, point) => hitTestBreakdown(frame.layout.layout, point[0], point[1], frame.binding.scrollTop),
  tooltip(frame, key, focus) {
    const rowKey = focus ? key : inspectedRow(key);
    const tip = rowKey ? buildBreakdownTooltip(frame.model, frame.layout.level, rowKey, frame.theme) : null;
    return tip && rowKey ? { hour: frame.model.hour, rowKey, content: tip } : null;
  },
  structure({ binding, model, layout }) {
    return {
      meta: {
        empty: model.empty ?? null,
        emptyDetail: model.emptyDetail ?? null,
        emptyKind: model.emptyKind ?? 'empty',
        warnings: model.warnings ?? [],
        contentHeight: layout.contentHeight,
        view: binding.options.view,
      },
      nav: layout.nav,
    };
  },
  mirror: frame => frame.layout.mirror,
  audit: frame => deriveBreakdownAudit(frame.model, frame.layout.layout),
  visibleArt: frame => collectArtUrls(walkBreakdownArt(frame.model.parts, [])),
};
