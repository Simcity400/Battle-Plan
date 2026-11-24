import { LABEL_WEIGHT } from '../../charts/draw.ts';
import { textWidth } from '../../charts/textWidth.ts';
import type { LaneView } from '../../draw/lane.ts';
import { createLaneView, type LaneBinding, type LaneFamily, type LaneFrame, type LaneRuntimeOptions } from '../laneRuntime.ts';
import { glideDurationForSpeed } from '../table/painter.ts';
import { barsAuditWindow, barsScrollHeight, layoutFamilyBars, visibleBarsRange, type BarsLayout } from './layout.ts';
import { assertBarsModelValid, deriveBarsAudit, deriveBarsMirror, deriveBarsTooltip, type BarsModel } from './model.ts';
import { collectVisibleBarsIcons, drawFamilyBars, hitFamilyBars, type BarsGlideOffset } from './painter.ts';
import { hitPickList, pickAudit, pickedKey, pickNav, pickTooltip } from './pickList.ts';
import { barsRegionRows } from './shareRows.ts';
import { regionHitKey } from '../regions/regionRows.ts';

export interface FamilyBarsBinding<TOptions = unknown> extends LaneBinding<TOptions> {
  readonly family: 'bars';
  readonly width: number;
  readonly height: number;
  readonly reducedMotion?: boolean;
  readonly pauseFlush?: number;
}

type BarsFrame = LaneFrame<FamilyBarsBinding, BarsModel, BarsLayout>;

interface Glide {
  readonly duration: number;
  readonly offsets: ReadonlyMap<string, BarsGlideOffset>;
  readonly moving: boolean;
}

const STILL: Glide = { duration: 0, offsets: new Map(), moving: false };
const glides = new WeakMap<BarsLayout, { readonly before: unknown; readonly offsets: ReadonlyMap<string, BarsGlideOffset> }>();

function glideOf(frame: BarsFrame): Glide {
  const { model, layout, before, binding } = frame;
  const duration = model.animateOrder ? glideDurationForSpeed(frame.speed, binding.reducedMotion) : 0;
  if (duration === 0 || !before || before.hour === frame.hour || frame.now - frame.changedAt >= duration) return { ...STILL, duration };
  let held = glides.get(layout);
  if (!held || held.before !== before) {
    const prior = new Map<string, number>();
    for (let i = 0; i < before.model.rows.length; i++) prior.set(before.model.rows[i]!.key, before.layout.bodyTop + before.layout.rowTops[i]! - before.binding.scrollTop);
    const offsets = new Map<string, BarsGlideOffset>();
    for (let i = 0; i < model.rows.length; i++) {
      const key = model.rows[i]!.key;
      const fromY = prior.get(key);
      const nextY = layout.bodyTop + layout.rowTops[i]! - binding.scrollTop;
      if (fromY !== undefined && Math.abs(fromY - nextY) > 1 && Math.abs(fromY - nextY) <= layout.bodyHeight) offsets.set(key, { fromY, startedAt: frame.changedAt });
    }
    glides.set(layout, held = { before, offsets });
  }
  return { duration, offsets: held.offsets, moving: held.offsets.size > 0 };
}

function inspected(inspect: string | null): { readonly key: string; readonly side?: 'right' } | null {
  if (inspect?.startsWith('row:')) return { key: inspect.slice(4) };
  if (inspect?.startsWith('right:')) return { key: inspect.slice(6), side: 'right' };
  if (inspect?.startsWith('pick:')) return { key: inspect };
  return null;
}

const barsLane: LaneFamily<FamilyBarsBinding, BarsModel, BarsLayout> = {
  family: 'bars',
  boundary(model, hour) {
    assertBarsModelValid(model, hour);
    return model.nextChangeAt ?? Infinity;
  },
  redraw: () => Infinity,
  status: model => (model.rows.length > 0 || model.empty !== undefined ? 'picture' : 'scene'),
  view: () => [],
  layout(model, _binding, surface, theme, measure) {
    const font = `${LABEL_WEIGHT} ${theme.fontSize.body}px ${theme.font}`;
    return layoutFamilyBars(model, surface.width, surface.height, theme, text => {
      measure.font = font;
      return textWidth(measure, text);
    });
  },
  paint(ctx, frame) {
    const glide = glideOf(frame);
    drawFamilyBars(ctx, {
      model: frame.model,
      layout: frame.layout,
      theme: frame.theme,
      scrollTop: frame.binding.scrollTop,
      now: frame.now,
      glideDuration: glide.duration,
      glide: glide.offsets,
      icons: frame.art.map,
      artCache: frame.art,
      inspectedKey: inspected(frame.inspect)?.key ?? null,
      selectedKey: frame.binding.selectedKey,
      focusedKey: frame.binding.focusedKey,
    });
    return glide.moving;
  },
  hit(frame, point) {
    if (frame.model.pickList) {
      const picked = hitPickList(frame.model.pickList, frame.layout, { x: point[0], y: point[1] });
      if (picked) return picked;
      if (point[0] < frame.layout.bodyLeft) return null;
    }
    const glide = glideOf(frame);
    const shown = new Set<string>();
    const hovered = inspected(frame.inspect)?.key;
    if (hovered) shown.add(hovered);
    if (frame.binding.focusedKey) shown.add(frame.binding.focusedKey);
    const res = hitFamilyBars(frame.layout, frame.model, { x: point[0], y: point[1] }, frame.binding.scrollTop, glide.moving ? glide.offsets : undefined, frame.now, glide.duration, shown);
    if (res.kind === 'none' || !res.rowKey) return null;
    if (res.kind === 'add-region') return regionHitKey('add', res.rowKey);
    if (res.kind === 'remove-region') return regionHitKey('remove', res.rowKey);
    if (res.kind === 'grip-region') return regionHitKey('grip', res.rowKey);
    if (res.kind === 'right-row') return `right:${res.rowKey}`;
    return `row:${res.rowKey}`;
  },
  tooltip(frame, key, focus) {
    const target = focus ? { key } : inspected(key);
    if (!target) return null;
    const picked = pickedKey(target.key);
    if (picked !== null) {
      const tip = frame.model.pickList ? pickTooltip(frame.model.pickList, picked, frame.model.hour) : null;
      return tip ? { hour: frame.model.hour, rowKey: target.key, payload: tip } : null;
    }
    const payload = deriveBarsTooltip(frame.model, target.key, target.side);
    return payload ? { hour: frame.model.hour, rowKey: target.key, payload } : null;
  },
  structure({ model, layout }) {
    const regions = barsRegionRows(model, layout);
    return {
      ...(regions ? { regions } : {}),
      meta: {
        form: model.form,
        count: model.rows.length,
        totalHeight: layout.totalHeight,
        scrollHeight: barsScrollHeight(layout),
        rowViewport: { top: layout.bodyTop, height: layout.bodyHeight },
        bodyHeight: layout.bodyHeight,
        rowHeight: layout.dataRowHeight,
        bodyTop: layout.bodyTop,
        summaryText: model.summaryText ?? null,
        empty: model.empty ?? null,
        emptyDetail: model.emptyDetail ?? null,
        emptyKind: model.emptyKind ?? 'empty',
        notes: model.notes ?? [],
        warnings: model.warnings ?? [],
      },
      nav: [
        ...(model.pickList && layout.pickWidth > 0 ? pickNav(model.pickList) : []),
        ...model.rows.flatMap((row, index) => (row.kind === 'section' ? [] : [[row.key, row.label, index, -1, '', row.kind === 'data' ? (row.meta ?? null) : null, layout.rowTops[index]!, layout.rowHeights[index]!] as const])),
      ],
    };
  },
  mirror(frame) {
    const shown = visibleBarsRange(frame.layout, frame.binding.scrollTop);
    return deriveBarsMirror(frame.model, { first: shown.first, count: Math.max(0, shown.last - shown.first) }, frame.binding.focusedKey);
  },
  audit: frame => [...(frame.model.pickList ? pickAudit(frame.model.pickList) : []), ...deriveBarsAudit(frame.model, { window: barsAuditWindow(frame.layout, frame.binding.scrollTop) })],
  visibleArt: frame => collectVisibleBarsIcons(frame.model, frame.layout, frame.binding.scrollTop),
};

export function familyBarsLaneView(wake: (widget: number) => void, options?: LaneRuntimeOptions): LaneView {
  return createLaneView(barsLane, wake, options);
}
