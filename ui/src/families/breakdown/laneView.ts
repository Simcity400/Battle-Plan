import type { LaneView } from '../../draw/lane.ts';
import type { MirrorModel } from '../contract.ts';
import type { FamilyNavTuple } from '../host/keyboard.ts';
import { createLaneView, type LaneBinding, type LaneFamily, type LaneRuntimeOptions } from '../laneRuntime.ts';
import type { BreakdownViewOptions } from './declaration.ts';
import {
  buildBreakdownTooltip,
  deriveBreakdownAudit,
  deriveBreakdownNavAndMirror,
  breakdownLevel,
  type BreakdownModel,
  type BreakdownPart,
  type ResolvedBreakdownLevel,
} from './model.ts';
import { hitTestBreakdown, layoutBreakdown, type BreakdownLayout } from './layout.ts';
import { drawBreakdown } from './painter.ts';

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

const measures = new WeakMap<OffscreenCanvasRenderingContext2D, Map<string, number>>();

function measurer(ctx: OffscreenCanvasRenderingContext2D): (text: string, font: string) => number {
  let cache = measures.get(ctx);
  if (!cache) measures.set(ctx, cache = new Map());
  const held = cache;
  return (text, font) => {
    const key = `${font}\u0000${text}`;
    let width = held.get(key);
    if (width === undefined) {
      if (held.size > 4096) held.clear();
      ctx.font = font;
      width = ctx.measureText(text).width;
      held.set(key, width);
    }
    return width;
  };
}

function collectArtUrls(parts: readonly BreakdownPart[], out: string[]): string[] {
  for (const part of parts) {
    if (part.flagArt?.status === 'ready' && part.flagArt.url) out.push(part.flagArt.url);
    if (part.children) collectArtUrls(part.children, out);
  }
  return out;
}

const inspectedRow = (inspect: string | null): string | null => (inspect?.startsWith('row:') ? inspect.slice(4) : null);

const breakdownLane: LaneFamily<FamilyBreakdownBinding, BreakdownModel, BreakdownFrameLayout> = {
  family: 'breakdown',
  boundary: () => Infinity,
  redraw: () => Infinity,
  status: () => 'scene',
  view: binding => [binding.expanded],
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
      measure: measurer(measure),
    });
    const contentHeight = layout.view === 'ring' ? (layout.sideList?.contentHeight ?? surface.height) : layout.contentHeight;
    const { nav, mirror } = deriveBreakdownNavAndMirror(level, layout.expanded ?? {});
    return { level, layout, contentHeight, nav, mirror };
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
  structure({ model, layout }) {
    return {
      meta: {
        empty: model.empty ?? null,
        emptyDetail: model.emptyDetail ?? null,
        emptyKind: model.emptyKind ?? 'empty',
        warnings: model.warnings ?? [],
        contentHeight: layout.contentHeight,
      },
      nav: layout.nav,
    };
  },
  mirror: frame => frame.layout.mirror,
  audit: frame => deriveBreakdownAudit(frame.model),
  visibleArt: frame => collectArtUrls(frame.model.parts, []),
};

export function familyBreakdownLaneView(wake: (widget: number) => void, options?: LaneRuntimeOptions): LaneView {
  return createLaneView(breakdownLane, wake, options);
}
