import { canvasMeasureText } from '../../charts/textWidth.ts';
import { collectArtUrls, type ArtUrlInput } from '../art/provider.ts';
import type { FamilyNavTuple } from '../host/keyboard.ts';
import type { LaneBinding, LaneFamily } from '../laneRuntime.ts';
import type { BreakdownViewOptions } from './declaration.ts';
import {
  buildBreakdownTooltip,
  breakdownLevel,
  type BreakdownModel,
  type ResolvedBreakdownLevel,
} from './model.ts';
import { firstRowFrom, hitTestBreakdown, layoutBreakdown, type BreakdownLayout } from './layout.ts';
import { drawBreakdown } from './painter.ts';
import { deriveBreakdownAudit, deriveBreakdownMirror, deriveBreakdownNav } from './structure.ts';

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
}

const drawnArt = new WeakMap<BreakdownFrameLayout, { readonly scrollTop: number; readonly urls: readonly string[] }>();

function drawnArtOf(frame: BreakdownFrameLayout, scrollTop: number): readonly string[] {
  const held = drawnArt.get(frame);
  if (held && held.scrollTop === scrollTop) return held.urls;
  const { layout } = frame;
  let urls: readonly string[] = [];
  if (layout.view === 'parts' && !layout.narrow && layout.height > layout.bodyTop) {
    const viewTop = layout.bodyTop;
    const shown: ArtUrlInput[] = [];
    for (let at = firstRowFrom(layout.rows, scrollTop); at < layout.rows.length; at++) {
      const row = layout.rows[at]!;
      const sy = viewTop + row.y - scrollTop;
      if (sy >= layout.height) break;
      if (row.depth < 2 && row.part.flagArt) shown.push(row.part.flagArt);
    }
    urls = collectArtUrls(shown);
  }
  drawnArt.set(frame, { scrollTop, urls });
  return urls;
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
    const nav = deriveBreakdownNav(layout);
    return { level, layout, contentHeight: layout.contentHeight, nav };
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
    return tip && rowKey ? { rowKey, payload: tip } : null;
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
  mirror: frame => deriveBreakdownMirror(frame.layout.layout, frame.layout.level.total),
  audit: frame => deriveBreakdownAudit(frame.model, frame.layout.layout),
  visibleArt: frame => drawnArtOf(frame.layout, frame.binding.scrollTop),
};
