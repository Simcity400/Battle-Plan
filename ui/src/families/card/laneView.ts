import type { LaneView } from '../../draw/lane.ts';
import { createLaneView, type LaneBinding, type LaneFamily, type LaneRuntimeOptions } from '../laneRuntime.ts';
import { layoutCards, type CardLayout } from './layout.ts';
import { assertCardModelValid, cardRowHours, deriveCardAudit, deriveCardMirror, deriveCardTooltip, type CardModel } from './model.ts';
import { drawCards, hitCards, visibleCardIcons } from './painter.ts';

export interface FamilyCardBinding<TOptions = unknown> extends LaneBinding<TOptions> {
  readonly family: 'card';
  readonly width: number;
  readonly height: number;
  readonly pauseFlush?: number;
}

const inspectedRow = (inspect: string | null): string | null => (inspect?.startsWith('row:') ? inspect.slice(4) : null);

const cardLane: LaneFamily<FamilyCardBinding, CardModel, CardLayout> = {
  family: 'card',
  boundary(model, hour) {
    assertCardModelValid(model, hour);
    return model.nextChangeAt ?? Infinity;
  },
  redraw: () => Infinity,
  status: model => (model.rows.length > 0 || model.empty !== undefined ? 'picture' : 'scene'),
  view: [],
  layout: (model, _binding, surface) => layoutCards(model, surface.width, surface.height),
  paint(ctx, frame) {
    drawCards(ctx, {
      model: frame.model,
      layout: frame.layout,
      theme: frame.theme,
      scrollTop: frame.binding.scrollTop,
      artCache: frame.art,
      inspectedKey: inspectedRow(frame.inspect),
      selectedKey: frame.binding.selectedKey,
      focusedKey: frame.binding.focusedKey,
    });
    return false;
  },
  hit(frame, point) {
    const key = hitCards(frame.layout, frame.model, { x: point[0], y: point[1] }, frame.binding.scrollTop);
    return key ? `row:${key}` : null;
  },
  tooltip(frame, key, focus) {
    const rowKey = focus ? key : inspectedRow(key);
    const payload = rowKey ? deriveCardTooltip(frame.model, rowKey) : null;
    return payload && rowKey ? { hour: frame.model.hour, rowKey, payload } : null;
  },
  structure({ model, layout }) {
    return {
      meta: {
        count: model.rows.length,
        totalHeight: layout.totalHeight,
        bodyHeight: layout.height,
        summaryText: model.summaryText ?? null,
        empty: model.empty ?? null,
        emptyDetail: null,
        emptyKind: model.emptyKind ?? 'empty',
        notes: model.notes ?? [],
        warnings: model.warnings ?? [],
      },
      rowHours: cardRowHours(model),
      nav: model.rows.flatMap((row, index) => (row.kind === 'section' ? [] : [[row.key, row.kind === 'hero' || row.title.kind !== 'known' ? row.key : String(row.title.value), index, -1, '', row.subject ?? null, layout.rowTops[index]!, layout.rowHeights[index]!] as const])),
    };
  },
  mirror: frame => deriveCardMirror(frame.model),
  audit: frame => deriveCardAudit(frame.model),
  visibleArt: frame => visibleCardIcons(frame.model, frame.layout, frame.binding.scrollTop),
};

export function familyCardLaneView(wake: (widget: number) => void, options?: LaneRuntimeOptions): LaneView {
  return createLaneView(cardLane, wake, options);
}
