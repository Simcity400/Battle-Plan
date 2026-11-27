import { LABEL_WEIGHT } from '../../charts/draw.ts';
import { textWidth } from '../../charts/textWidth.ts';
import type { LaneBinding, LaneFamily, LaneFrame } from '../laneRuntime.ts';
import { interpolateFamilyValue } from '../motion.ts';
import { glideDurationForSpeed } from '../table/painter.ts';
import { valueNumber } from '../value.ts';
import { formatValueState } from '../valueFormat.ts';
import { barsAuditWindow, barsRangeForBand, barsScales, barsScrollHeight, layoutFamilyBars, visibleBarsRange, type BarsLayout } from './layout.ts';
import { assertBarsModelValid, barsLabelText, deriveBarsAudit, deriveBarsMirror, deriveBarsTooltip, formatBarsValueField, type BarsDataRow, type BarsModel, type BarsValueField } from './model.ts';
import { collectVisibleBarsIcons, drawFamilyBars, easeBarFraction, easeOutCubic, hitFamilyBars, type BarsEasedRow, type BarsGlideOffset } from './painter.ts';
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
const paintedMotionY = new WeakMap<BarsFrame, ReadonlyMap<string, number>>();

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
    if (isMeasuredField(row.value) && row.segments?.length) {
      const shares: Record<string, number> = {};
      for (const seg of row.segments) {
        if (!isMeasuredField({ value: seg.value })) continue;
        shares[seg.key] = seg.share;
      }
      if (Object.keys(shares).length > 0) {
        out.segmentShares = shares;
      }
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
      if (prev.segmentShares) {
        const shares: Record<string, number> = {};
        for (const [segKey, curShare] of Object.entries(cur.segmentShares)) {
          const prevShare = prev.segmentShares[segKey];
          const val = step(prevShare, curShare);
          shares[segKey] = val ?? curShare;
        }
        next.segmentShares = shares;
      } else {
        next.segmentShares = cur.segmentShares;
      }
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

function formatInterpolatedBarsField(field: BarsValueField, interp: number, origNumeric: number): string | undefined {
  if (Math.abs(interp - origNumeric) <= 1e-6) {
    const fmt = formatBarsValueField(field);
    return fmt.kind === 'known' || fmt.kind === 'zero' || fmt.kind === 'partial' ? fmt.text : undefined;
  }
  const value = interpolateFamilyValue(field.value, interp, { originalNumeric: origNumeric, format: field.format });
  const fmt = formatBarsValueField({ ...field, value });
  return fmt.kind === 'known' || fmt.kind === 'zero' || fmt.kind === 'partial' ? fmt.text : undefined;
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
    const liveMotion = frame.motion;
    if (liveMotion) {
      const { field, step, nextModel } = liveMotion;
      const { model, layout, binding } = frame;
      const visible = visibleBarsRange(layout, binding.scrollTop, 2);
      const nextRows = nextModel && nextModel.form === model.form
        ? new Map<string, BarsDataRow>(nextModel.rows.filter((r): r is BarsDataRow => r.kind === 'data').map(r => [r.key, r]))
        : null;
      const nextScales = nextModel && nextModel.form === model.form
        ? barsScales(nextModel, layout.showAxis)
        : null;
      const interpScaleMax = field.stepScalar('scaleMax', layout.scaleMax, nextScales?.scaleMax, false, easeOutCubic) ?? layout.scaleMax;
      const interpScaleLeft = field.stepScalar('scaleLeftMax', layout.scaleLeftMax, nextScales?.scaleLeftMax, false, easeOutCubic) ?? layout.scaleLeftMax;
      const interpScaleRight = field.stepScalar('scaleRightMax', layout.scaleRightMax, nextScales?.scaleRightMax, false, easeOutCubic) ?? layout.scaleRightMax;
      const safeScale = Math.max(1e-6, interpScaleMax);
      const safeLeftScale = Math.max(1e-6, interpScaleLeft);
      const safeRightScale = Math.max(1e-6, interpScaleRight);
      const activeScalars = new Set<string>(['scaleMax', 'scaleLeftMax', 'scaleRightMax']);
      const activeYs = new Set<string>();
      const easedMap = step.mode !== 'exact' ? new Map<string, BarsEasedRow>() : undefined;
      const hitYMap = step.mode !== 'exact' ? new Map<string, number>() : undefined;

      for (let i = visible.first; i < visible.last; i++) {
        const item = model.rows[i]!;
        const baseY = layout.bodyTop + layout.rowTops[i]! - binding.scrollTop;
        const yKey = `y:${item.key}`;
        activeYs.add(yKey);
        const yOverride = field.stepY(yKey, baseY, layout.bodyHeight, easeOutCubic);
        if (hitYMap) hitYMap.set(item.key, yOverride);
        if (item.kind !== 'data') continue;
        const h1Row = nextRows?.get(item.key);
        const rowEased: {
          numericFrac?: number;
          leftFrac?: number;
          rightFrac?: number;
          rightSideFrac?: number;
          targetFrac?: number;
          segmentShares?: Record<string, number>;
          valueText?: string;
          leftText?: string;
          rightText?: string;
          rightSideText?: string;
          pairTopText?: string;
          pairBottomText?: string;
          yOverride?: number;
        } = { yOverride };

        if (model.form === 'share' || model.form === 'stacked') {
          const parentMeasured = isMeasuredField(item.value);
          if (item.segments?.length) {
            const nextSegs = h1Row?.segments ? new Map(h1Row.segments.map(s => [s.key, s])) : null;
            const shares: Record<string, number> = {};
            for (const seg of item.segments) {
              const sKey = `seg:${item.key}:${seg.key}`;
              activeScalars.add(sKey);
              const segMeasured = parentMeasured && isMeasuredField({ value: seg.value });
              const h1Seg = nextSegs?.get(seg.key);
              const h1Share = h1Seg
                ? (isMeasuredField({ value: h1Seg.value }) ? h1Seg.share : null)
                : (h1Row ? 0 : undefined);
              const interpShare = field.stepScalar(sKey, segMeasured ? seg.share : null, h1Share, !segMeasured, easeOutCubic);
              if (easedMap && segMeasured && interpShare !== null && Number.isFinite(interpShare)) {
                shares[seg.key] = Math.max(0, Math.min(1, interpShare));
              }
            }
            if (easedMap && Object.keys(shares).length > 0) rowEased.segmentShares = shares;
          }
        } else if (model.form === 'two-sided' && layout.twoSidedLayout === 'split-sides') {
          if (item.value) {
            const vKey = `v:${item.key}`;
            activeScalars.add(vKey);
            const measured = isMeasuredField(item.value) && item.numeric !== null && item.numeric !== undefined;
            const h1Val = h1Row?.value
              ? (isMeasuredField(h1Row.value) && h1Row.numeric !== null && h1Row.numeric !== undefined ? h1Row.numeric : null)
              : undefined;
            const interpVal = field.stepScalar(vKey, measured ? item.numeric! : null, h1Val, !measured, easeOutCubic);
            if (easedMap && measured && interpVal !== null && Number.isFinite(interpVal)) {
              rowEased.numericFrac = interpVal / safeLeftScale;
              const text = formatInterpolatedBarsField(item.value, interpVal, item.numeric!);
              if (text !== undefined) rowEased.valueText = text;
            }
          }
          if (item.rightSide) {
            const rsKey = `rs:${item.rightSide.key}`;
            activeScalars.add(rsKey);
            const rsMeasured = isMeasuredField(item.rightSide.value) && item.rightSide.numeric !== null && item.rightSide.numeric !== undefined;
            const h1Right = h1Row?.rightSide;
            const h1Val = h1Right
              ? (isMeasuredField(h1Right.value) && h1Right.numeric !== null && h1Right.numeric !== undefined ? h1Right.numeric : null)
              : undefined;
            const interpVal = field.stepScalar(rsKey, rsMeasured ? item.rightSide.numeric! : null, h1Val, !rsMeasured, easeOutCubic);
            if (easedMap && rsMeasured && interpVal !== null && Number.isFinite(interpVal)) {
              rowEased.rightSideFrac = interpVal / safeRightScale;
              const text = formatInterpolatedBarsField(item.rightSide.value, interpVal, item.rightSide.numeric!);
              if (text !== undefined) rowEased.rightSideText = text;
            }
          }
        } else if (model.form === 'two-sided') {
          const leftDenom = layout.twoSidedLayout === 'stacked-pair' ? safeScale : safeLeftScale;
          const rightDenom = layout.twoSidedLayout === 'stacked-pair' ? safeScale : safeRightScale;
          if (item.left) {
            const lKey = `l:${item.key}`;
            activeScalars.add(lKey);
            const lMeasured = isMeasuredField(item.left.value) && item.left.numeric !== null && item.left.numeric !== undefined;
            const h1Left = h1Row?.left;
            const h1Val = h1Left
              ? (isMeasuredField(h1Left.value) && h1Left.numeric !== null && h1Left.numeric !== undefined ? h1Left.numeric : null)
              : undefined;
            const interpVal = field.stepScalar(lKey, lMeasured ? item.left.numeric! : null, h1Val, !lMeasured, easeOutCubic);
            if (easedMap && lMeasured && interpVal !== null && Number.isFinite(interpVal)) {
              rowEased.leftFrac = interpVal / leftDenom;
              const text = formatInterpolatedBarsField(item.left.value, interpVal, item.left.numeric!);
              if (text !== undefined) rowEased.leftText = text;
            }
          }
          if (item.right) {
            const rKey = `r:${item.key}`;
            activeScalars.add(rKey);
            const rMeasured = isMeasuredField(item.right.value) && item.right.numeric !== null && item.right.numeric !== undefined;
            const h1Right = h1Row?.right;
            const h1Val = h1Right
              ? (isMeasuredField(h1Right.value) && h1Right.numeric !== null && h1Right.numeric !== undefined ? h1Right.numeric : null)
              : undefined;
            const interpVal = field.stepScalar(rKey, rMeasured ? item.right.numeric! : null, h1Val, !rMeasured, easeOutCubic);
            if (easedMap && rMeasured && interpVal !== null && Number.isFinite(interpVal)) {
              rowEased.rightFrac = interpVal / rightDenom;
              const text = formatInterpolatedBarsField(item.right.value, interpVal, item.right.numeric!);
              if (text !== undefined) rowEased.rightText = text;
            }
          }
        } else {
          const vKey = `v:${item.key}`;
          activeScalars.add(vKey);
          const valField = item.value;
          const measured = valField !== undefined && isMeasuredField(valField) && item.numeric !== null && item.numeric !== undefined;
          const h1Val = h1Row
            ? (isMeasuredField(h1Row.value) && h1Row.numeric !== null && h1Row.numeric !== undefined ? h1Row.numeric : null)
            : undefined;
          const interpVal = field.stepScalar(vKey, measured ? item.numeric! : null, h1Val, !measured, easeOutCubic);
          if (easedMap && measured && valField && interpVal !== null && Number.isFinite(interpVal)) {
            rowEased.numericFrac = interpVal / safeScale;
            const text = formatInterpolatedBarsField(valField, interpVal, item.numeric!);
            if (text !== undefined) rowEased.valueText = text;
          }
          if (item.target) {
            const tKey = `t:${item.key}`;
            activeScalars.add(tKey);
            const tMeasured = isMeasuredField({ value: item.target.value }) && Number.isFinite(item.target.numeric);
            const h1Target = h1Row?.target;
            const h1TargetVal = h1Target
              ? (isMeasuredField({ value: h1Target.value }) && Number.isFinite(h1Target.numeric) ? h1Target.numeric : null)
              : undefined;
            const interpTarget = field.stepScalar(tKey, tMeasured ? item.target.numeric : null, h1TargetVal, !tMeasured, easeOutCubic);
            if (easedMap && tMeasured && interpTarget !== null && Number.isFinite(interpTarget)) {
              rowEased.targetFrac = Math.max(0, interpTarget / safeScale);
            }
          }
          if (item.segments?.length) {
            const nextSegs = h1Row?.segments ? new Map(h1Row.segments.map(s => [s.key, s])) : null;
            const shares: Record<string, number> = {};
            for (const seg of item.segments) {
              const sKey = `seg:${item.key}:${seg.key}`;
              activeScalars.add(sKey);
              const segMeasured = isMeasuredField({ value: seg.value });
              const h1Seg = nextSegs?.get(seg.key);
              const h1Share = h1Seg ? (isMeasuredField({ value: h1Seg.value }) ? h1Seg.share : null) : (h1Row ? 0 : undefined);
              const interpShare = field.stepScalar(sKey, segMeasured ? seg.share : null, h1Share, !segMeasured, easeOutCubic);
              if (easedMap && segMeasured && interpShare !== null && Number.isFinite(interpShare)) {
                shares[seg.key] = Math.max(0, Math.min(1, interpShare));
              }
            }
            if (easedMap && Object.keys(shares).length > 0) rowEased.segmentShares = shares;
          }
          if (model.form === 'net' && item.pair) {
            const ptKey = `pt:${item.key}`;
            const pbKey = `pb:${item.key}`;
            activeScalars.add(ptKey);
            activeScalars.add(pbKey);
            const topNum = valueNumber(item.pair.topValue);
            const botNum = valueNumber(item.pair.bottomValue);
            const h1Top = h1Row?.pair ? (valueNumber(h1Row.pair.topValue) ?? null) : undefined;
            const h1Bot = h1Row?.pair ? (valueNumber(h1Row.pair.bottomValue) ?? null) : undefined;
            const interpTop = field.stepScalar(ptKey, topNum ?? null, h1Top, topNum === undefined, easeOutCubic);
            const interpBot = field.stepScalar(pbKey, botNum ?? null, h1Bot, botNum === undefined, easeOutCubic);
            if (easedMap && topNum !== undefined && interpTop !== null && Number.isFinite(interpTop)) {
              const nextVal = interpolateFamilyValue(item.pair.topValue, interpTop, { format: item.pair.format });
              const fmt = formatValueState(nextVal, item.pair.format);
              if (fmt.kind === 'known' || fmt.kind === 'zero' || fmt.kind === 'partial') rowEased.pairTopText = fmt.text;
            }
            if (easedMap && botNum !== undefined && interpBot !== null && Number.isFinite(interpBot)) {
              const nextVal = interpolateFamilyValue(item.pair.bottomValue, interpBot, { format: item.pair.format });
              const fmt = formatValueState(nextVal, item.pair.format);
              if (fmt.kind === 'known' || fmt.kind === 'zero' || fmt.kind === 'partial') rowEased.pairBottomText = fmt.text;
            }
          }
        }
        if (easedMap) easedMap.set(item.key, rowEased);
      }
      if (hitYMap) paintedMotionY.set(frame, hitYMap);
      else paintedMotionY.delete(frame);
      field.prune(activeScalars, activeYs);
      drawFamilyBars(ctx, {
        model: frame.model,
        layout: frame.layout,
        theme: frame.theme,
        scrollTop: frame.binding.scrollTop,
        now: frame.now,
        glideDuration: 0,
        ...(easedMap ? { easedValues: easedMap } : {}),
        artCache: frame.art,
        inspectedKey: inspected(frame.inspect)?.key ?? null,
        selectedKey: frame.binding.selectedKey,
        focusedKey: frame.binding.focusedKey,
      });
      return field.active;
    }
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
    const motionY = paintedMotionY.get(frame);
    const glide = motionY ? STILL : glideOf(frame);
    const shown = new Set<string>();
    const hovered = inspected(frame.inspect)?.key;
    if (hovered) shown.add(hovered);
    if (frame.binding.focusedKey) shown.add(frame.binding.focusedKey);
    const res = hitFamilyBars(frame.layout, frame.model, { x: point[0], y: point[1] }, frame.binding.scrollTop, glide.moving ? glide.offsets : undefined, frame.now, glide.duration, shown, motionY);
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
    const dataHour = frame.model.dataHours?.length === 1 ? frame.model.dataHours[0]! : undefined;
    const tipHour = dataHour ?? frame.model.hour;
    if (picked !== null) {
      const tip = frame.model.pickList ? pickTooltip(frame.model.pickList, picked, tipHour) : null;
      if (!tip) return null;
      const fixedHour = dataHour ?? (tip.hour !== undefined && tip.hour !== frame.model.hour ? tip.hour : undefined);
      return { rowKey: target.key, payload: tip, ...(fixedHour !== undefined ? { hour: fixedHour } : {}) };
    }
    const payload = deriveBarsTooltip(frame.model, target.key, target.side);
    if (!payload) return null;
    const fixedHour = dataHour ?? (payload.hour !== undefined && payload.hour !== frame.model.hour ? payload.hour : undefined);
    return { rowKey: target.key, payload, ...(fixedHour !== undefined ? { hour: fixedHour } : {}) };
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
        ...model.rows.flatMap((row, index) => (row.kind === 'section' ? [] : [[row.key, row.kind === 'data' ? barsLabelText(row.label) : row.label, index, -1, '', row.kind === 'data' ? (row.meta ?? null) : null, layout.rowTops[index]!, layout.rowHeights[index]!] as const])),
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
