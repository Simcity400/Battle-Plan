import { LABEL_WEIGHT } from '../../charts/draw.ts';
import { textWidth } from '../../charts/textWidth.ts';
import type { LaneBinding, LaneFamily, LaneFrame } from '../laneRuntime.ts';
import { glideDurationForSpeed } from '../table/painter.ts';
import { barsAuditWindow, barsRangeForBand, barsScrollHeight, layoutFamilyBars, visibleBarsRange, type BarsLayout } from './layout.ts';
import { assertBarsModelValid, deriveBarsAudit, deriveBarsMirror, deriveBarsTooltip, formatBarsValueField, type BarsDataRow, type BarsModel, type BarsValueField } from './model.ts';
import { collectVisibleBarsIcons, drawFamilyBars, easeBarFraction, hitFamilyBars, type BarsEasedRow, type BarsGlideOffset } from './painter.ts';
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
const glides = new WeakMap<BarsLayout, { readonly continuity: number; readonly scrollTop: number; readonly offsets: ReadonlyMap<string, BarsGlideOffset> }>();

function glideOf(frame: BarsFrame): Glide {
  const { model, layout, before, binding } = frame;
  const duration = model.animateOrder ? glideDurationForSpeed(frame.speed, binding.reducedMotion, binding.motion?.glideDurationMs) : 0;
  if (duration === 0 || !before || before.hour === frame.hour || frame.now - frame.changedAt >= duration) return { ...STILL, duration };
  let held = glides.get(layout);
  if (!held || held.continuity !== frame.continuity || held.scrollTop !== binding.scrollTop) {
    const visible = visibleBarsRange(layout, binding.scrollTop, 0);
    const bandTop = binding.scrollTop - layout.bodyHeight;
    const bandBottom = binding.scrollTop + layout.bodyHeight * 2;
    const priorRange = barsRangeForBand(before.layout, bandTop, bandBottom, 0);
    const prior = new Map<string, number>();
    for (let i = priorRange.first; i < priorRange.last; i++) {
      prior.set(before.model.rows[i]!.key, before.layout.rowTops[i]!);
    }
    const offsets = new Map<string, BarsGlideOffset>();
    for (let i = visible.first; i < visible.last; i++) {
      const key = model.rows[i]!.key;
      const beforeRowTop = prior.get(key);
      const nextRowTop = layout.rowTops[i]!;
      if (beforeRowTop !== undefined && Math.abs(beforeRowTop - nextRowTop) > 1 && Math.abs(beforeRowTop - nextRowTop) <= layout.bodyHeight) {
        offsets.set(key, { fromY: layout.bodyTop + beforeRowTop - binding.scrollTop, startedAt: frame.changedAt });
      }
    }
    glides.set(layout, held = { continuity: frame.continuity, scrollTop: binding.scrollTop, offsets });
  }
  return { duration, offsets: held.offsets, moving: held.offsets.size > 0 };
}

function isMeasuredField(field: BarsValueField | undefined): boolean {
  if (!field) return true;
  const kind = formatBarsValueField(field).kind;
  return kind !== 'notRecorded' && kind !== 'notApplicable';
}

function rowFractions(row: BarsDataRow, form: BarsModel['form'], layout: BarsLayout): BarsEasedRow {
  const scaleMax = Math.max(1e-6, layout.scaleMax);
  const scaleLeft = Math.max(1e-6, layout.scaleLeftMax);
  const scaleRight = Math.max(1e-6, layout.scaleRightMax);
  const out: {
    numericFrac?: number;
    leftFrac?: number;
    rightFrac?: number;
    rightSideFrac?: number;
    targetFrac?: number;
    segmentShares?: Record<string, number>;
  } = {};
  if (form === 'share' || form === 'stacked') {
    if (row.segments?.length) {
      const shares: Record<string, number> = {};
      for (const seg of row.segments) {
        shares[seg.key] = seg.share;
      }
      out.segmentShares = shares;
    }
  } else if (form === 'two-sided' && layout.twoSidedLayout === 'split-sides') {
    if (isMeasuredField(row.value) && row.numeric !== undefined && row.numeric !== null) {
      out.numericFrac = row.numeric / scaleLeft;
    }
    if (row.rightSide && isMeasuredField(row.rightSide.value) && row.rightSide.numeric !== undefined && row.rightSide.numeric !== null) {
      out.rightSideFrac = row.rightSide.numeric / scaleRight;
    }
  } else if (form === 'two-sided') {
    const leftScale = layout.twoSidedLayout === 'stacked-pair' ? scaleMax : scaleLeft;
    const rightScale = layout.twoSidedLayout === 'stacked-pair' ? scaleMax : scaleRight;
    if (row.left && isMeasuredField(row.left.value) && row.left.numeric !== undefined && row.left.numeric !== null) {
      out.leftFrac = row.left.numeric / leftScale;
    }
    if (row.right && isMeasuredField(row.right.value) && row.right.numeric !== undefined && row.right.numeric !== null) {
      out.rightFrac = row.right.numeric / rightScale;
    }
  } else {
    if (isMeasuredField(row.value) && row.numeric !== undefined && row.numeric !== null) {
      out.numericFrac = row.numeric / scaleMax;
    }
    if (row.target && row.target.numeric > 0) {
      out.targetFrac = row.target.numeric / scaleMax;
    }
  }
  return out;
}

function easedBarsOf(frame: BarsFrame): { readonly map?: ReadonlyMap<string, BarsEasedRow>; readonly moving: boolean } {
  const motion = frame.binding.motion;
  if (!motion?.easeValues || frame.binding.reducedMotion) return { moving: false };
  const duration = glideDurationForSpeed(frame.speed, false, motion.glideDurationMs);
  if (duration <= 0 || !frame.before || frame.before.hour === frame.hour || frame.before.model.form !== frame.model.form) {
    return { moving: false };
  }
  const progress = (frame.now - frame.changedAt) / duration;
  if (progress >= 1) return { moving: false };
  const prior = new Map<string, BarsEasedRow>();
  for (const item of frame.before.model.rows) {
    if (item.kind === 'data') {
      prior.set(item.key, rowFractions(item, frame.before.model.form, frame.before.layout));
    }
  }
  const eased = new Map<string, BarsEasedRow>();
  let moving = false;
  for (const item of frame.model.rows) {
    if (item.kind !== 'data') continue;
    const prev = prior.get(item.key);
    if (!prev) continue;
    const cur = rowFractions(item, frame.model.form, frame.layout);
    const next: {
      numericFrac?: number;
      leftFrac?: number;
      rightFrac?: number;
      rightSideFrac?: number;
      targetFrac?: number;
      segmentShares?: Record<string, number>;
    } = {};
    const step = (p: number | undefined, c: number | undefined): number | undefined => {
      if (p === undefined || c === undefined) return c;
      if (Math.abs(p - c) > 1e-4) moving = true;
      return easeBarFraction(p, c, progress) ?? c;
    };
    const nf = step(prev.numericFrac, cur.numericFrac);
    if (nf !== undefined) next.numericFrac = nf;
    const lf = step(prev.leftFrac, cur.leftFrac);
    if (lf !== undefined) next.leftFrac = lf;
    const rf = step(prev.rightFrac, cur.rightFrac);
    if (rf !== undefined) next.rightFrac = rf;
    const rsf = step(prev.rightSideFrac, cur.rightSideFrac);
    if (rsf !== undefined) next.rightSideFrac = rsf;
    const tf = step(prev.targetFrac, cur.targetFrac);
    if (tf !== undefined) next.targetFrac = tf;
    if (cur.segmentShares) {
      const shares: Record<string, number> = {};
      for (const [segKey, curShare] of Object.entries(cur.segmentShares)) {
        const prevShare = prev.segmentShares?.[segKey] ?? 0;
        const val = step(prevShare, curShare);
        shares[segKey] = val ?? curShare;
      }
      next.segmentShares = shares;
    }
    eased.set(item.key, next);
  }
  return moving ? { map: eased, moving: true } : { moving: false };
}

function inspected(inspect: string | null): { readonly key: string; readonly side?: 'right' } | null {
  if (inspect?.startsWith('row:')) return { key: inspect.slice(4) };
  if (inspect?.startsWith('right:')) return { key: inspect.slice(6), side: 'right' };
  if (inspect?.startsWith('pick:')) return { key: inspect };
  return null;
}

export const barsLane: LaneFamily<FamilyBarsBinding, BarsModel, BarsLayout> = {
  family: 'bars',
  boundary(model, hour) {
    assertBarsModelValid(model, hour);
    return model.nextChangeAt ?? Infinity;
  },
  redraw: () => Infinity,
  status: model => (model.rows.length > 0 || model.empty !== undefined ? 'picture' : 'scene'),
  view: [],
  layout(model, _binding, surface, theme, measure) {
    const font = `${LABEL_WEIGHT} ${theme.fontSize.body}px ${theme.font}`;
    return layoutFamilyBars(model, surface.width, surface.height, theme, text => {
      measure.font = font;
      return textWidth(measure, text);
    });
  },
  paint(ctx, frame) {
    const glide = glideOf(frame);
    const eased = easedBarsOf(frame);
    drawFamilyBars(ctx, {
      model: frame.model,
      layout: frame.layout,
      theme: frame.theme,
      scrollTop: frame.binding.scrollTop,
      now: frame.now,
      glideDuration: glide.duration,
      glide: glide.offsets,
      ...(eased.map ? { easedValues: eased.map } : {}),
      artCache: frame.art,
      inspectedKey: inspected(frame.inspect)?.key ?? null,
      selectedKey: frame.binding.selectedKey,
      focusedKey: frame.binding.focusedKey,
    });
    return glide.moving || eased.moving;
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
    const tipHour = frame.model.dataHours?.length === 1 ? frame.model.dataHours[0]! : frame.model.hour;
    if (picked !== null) {
      const tip = frame.model.pickList ? pickTooltip(frame.model.pickList, picked, tipHour) : null;
      return tip ? { rowKey: target.key, payload: tip, ...(tip.hour !== undefined && tip.hour !== frame.model.hour ? { hour: tip.hour } : {}) } : null;
    }
    const payload = deriveBarsTooltip(frame.model, target.key, target.side);
    return payload ? { rowKey: target.key, payload, ...(payload.hour !== undefined && payload.hour !== frame.model.hour ? { hour: payload.hour } : {}) } : null;
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
  audit: frame => [...(frame.model.pickList && frame.layout.pickWidth > 0 ? pickAudit(frame.model.pickList) : []), ...deriveBarsAudit(frame.model, { window: barsAuditWindow(frame.layout, frame.binding.scrollTop) })],
  visibleArt: frame => collectVisibleBarsIcons(frame.model, frame.layout, frame.binding.scrollTop),
};
