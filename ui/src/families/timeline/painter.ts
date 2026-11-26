import { fitLine } from '../../charts/labels.ts';
import { textWidth, type Canvas2D } from '../../charts/textWidth.ts';
import { formatDateOnly } from '../valueFormat.ts';
import {
  computeTimeAxisTicks,
  drawChevron,
  drawGapBand,
  drawLegendStrip,
  drawRowFocusRing,
  drawRowHairline,
  drawRowHover,
  drawRowSelected,
  drawScrollFade,
  drawTimeAxis,
  drawTimeAxisGrid,
  drawTimelineBar,
  type TimeAxisTick,
} from '../../draw/kit.ts';
import { drawMediaSlot, resolveCachedArt, toArtSlotState } from '../art/adapters.ts';
import type { LaneArtCache } from '../art/laneCache.ts';
import { NO_ART } from '../art/types.ts';
import type { FamilyTheme } from '../contract.ts';
import {
  BAR_HEIGHT,
  BAR_HEIGHT_ICONS,
  GROUP_RULE_HEIGHT,
  hourToX,
  LANE_ICON,
  LEGEND_HEIGHT,
  METERS_HEADER_HEIGHT,
  METERS_PER_ROW,
  METERS_ROW_HEIGHT,
  placedLaneAt,
  TOP_AXIS_HEIGHT,
  visibleRange,
  xToHour,
  type PlacedLane,
  type TimelineLayout,
} from './layout.ts';
import { laneOpen, type TimelineHit, type TimelineLane, type TimelineModel, type TimelineSideLabel, type TimelineSpan } from './model.ts';
import { coalescedGaps, paintedSpans, spanUnder, type TimelineResolution } from './resolution.ts';

export interface TimelinePaintInput {
  readonly model: TimelineModel;
  readonly layout: TimelineLayout;
  readonly artCache?: LaneArtCache;
  readonly theme: FamilyTheme;
  readonly scrollTop: number;
  readonly expanded: Readonly<Record<string, boolean>>;
  readonly inspectedKey: string | null;
  readonly selectedKey: string | null;
  readonly focusedKey: string | null;
  readonly resolution: TimelineResolution;
}

const LABEL_PAD = 8;
const HIT_SLACK = 3;
const CHEVRON = 10;
const INDENT = 18;
const FLAG_W = 20;
const FLAG_H = 13;
const STRIP_W = 14;
const STRIP_H = 9;
const LOSER_ALPHA = 0.38;
const HALF = 6;
const DIAMOND = 12;
const MARK = 14;
const OUTCOME_GAP = 16;


function drawLabel(ctx: Canvas2D, placed: PlacedLane, y: number, input: TimelinePaintInput): void {
  const { lane } = placed;
  const { layout, theme, expanded, artCache } = input;
  const mid = y + placed.height / 2;
  let x = LABEL_PAD + (lane.depth ?? 0) * INDENT;
  const right = layout.labelWidth;
  if (lane.expandable) {
    drawChevron(ctx, x - 4, mid, laneOpen(lane, expanded), theme);
    x += CHEVRON + 6;
  }
  if (lane.flag) {
    drawMediaSlot(ctx, x, mid, FLAG_W, FLAG_H, { kind: 'flag', art: lane.flag, ...(lane.flagTag ? { tag: lane.flagTag } : {}) }, theme, { artCache });
    x += FLAG_W + 8;
  } else if (lane.icon) {
    drawMediaSlot(ctx, x, mid, LANE_ICON, LANE_ICON, { kind: 'art', art: lane.icon, name: lane.label }, theme, { artCache, radius: 3 });
    x += LANE_ICON + 8;
  } else if (lane.swatch) {
    ctx.save();
    ctx.fillStyle = lane.swatch;
    ctx.beginPath();
    ctx.roundRect(x, Math.round(mid - 6), 12, 12, 2.5);
    ctx.fill();
    ctx.restore();
    x += 12 + 8;
  }
  const width = Math.max(0, right - x);
  if (lane.sides) {
    if (lane.sidesBelowLabel && !layout.compact) {
      ctx.save();
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.font = `500 ${theme.fontSize.body}px ${theme.font}`;
      ctx.fillStyle = theme.text;
      ctx.fillText(fitLine(lane.label, width, s => textWidth(ctx, s)).text, x, mid - 9);
      ctx.restore();
      drawSides(ctx, lane.sides, x, right, mid + 9, input, {
        separator: lane.sidesSeparator,
        extraSides: lane.extraSides,
        smallFlags: true,
      });
      return;
    }
    drawSides(ctx, lane.sides, x, right, lane.subtitle && !layout.compact ? mid - 7 : mid, input, {
      separator: lane.sidesSeparator,
      extraSides: lane.extraSides,
      smallFlags: lane.sidesBelowLabel,
    });
    if (lane.subtitle && !layout.compact) {
      ctx.save();
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.font = `400 ${theme.fontSize.small}px ${theme.font}`;
      ctx.fillStyle = theme.muted;
      ctx.fillText(fitLine(lane.subtitle, width, s => textWidth(ctx, s)).text, x, mid + 8);
      ctx.restore();
    }
    return;
  }
  if (lane.kind === 'heading') {
    ctx.save();
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    ctx.font = `400 ${theme.fontSize.small}px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.fillText(fitLine(lane.label, width, s => textWidth(ctx, s)).text, x, mid);
    ctx.restore();
    return;
  }
  const twoLines = !!lane.subtitle && !layout.compact;
  ctx.save();
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  ctx.font = `500 ${theme.fontSize.body}px ${theme.font}`;
  ctx.fillStyle = theme.text;
  const title = fitLine(lane.label, width, s => textWidth(ctx, s)).text;
  ctx.fillText(title, x, twoLines ? mid - 7 : mid);
  if (lane.aside && !twoLines) {
    const after = x + textWidth(ctx, title) + 8;
    if (right - after > 24) {
      ctx.font = `400 ${theme.fontSize.small}px ${theme.font}`;
      ctx.fillStyle = theme.muted;
      ctx.fillText(fitLine(lane.aside, right - after, s => textWidth(ctx, s)).text, after, mid);
    }
  }
  if (twoLines) {
    ctx.font = `400 ${theme.fontSize.small}px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.fillText(fitLine(lane.subtitle!, width, s => textWidth(ctx, s)).text, x, mid + 8);
  }
  ctx.restore();
}

function drawSide(
  ctx: Canvas2D,
  side: TimelineSideLabel,
  x: number,
  right: number,
  mid: number,
  input: TimelinePaintInput,
  leadName: boolean,
  stripLimit = right,
  flagSize: readonly [number, number] = [FLAG_W, FLAG_H],
): number {
  const { theme, artCache } = input;
  const [fw, fh] = flagSize;
  ctx.save();
  if (side.dim) ctx.globalAlpha = LOSER_ALPHA;
  if (x + fw <= right) {
    drawMediaSlot(ctx, x, mid, fw, fh, { kind: 'flag', art: side.flag, tag: side.flagTag }, theme, { artCache });
    x += fw + 6;
  }
  if (leadName) {
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    ctx.font = `500 ${theme.fontSize.body}px ${theme.font}`;
    ctx.fillStyle = theme.text;
    const fitted = fitLine(side.name, Math.max(0, right - x), s => textWidth(ctx, s)).text;
    ctx.fillText(fitted, x, mid);
    x += textWidth(ctx, fitted) + 6;
  }
  for (const member of side.strip) {
    if (x + STRIP_W > Math.min(right, stripLimit)) break;
    drawMediaSlot(ctx, x, mid, STRIP_W, STRIP_H, { kind: 'flag', art: member.flag, tag: member.tag }, theme, { artCache });
    x += STRIP_W + 2;
  }
  ctx.restore();
  return x;
}

interface DrawSidesStyle {
  readonly separator?: string;
  readonly extraSides?: number;
  readonly smallFlags?: boolean;
}

function drawSides(
  ctx: Canvas2D,
  sides: readonly [TimelineSideLabel, TimelineSideLabel],
  x: number,
  right: number,
  mid: number,
  input: TimelinePaintInput,
  style: DrawSidesStyle = {},
): void {
  const { theme } = input;
  const names = !input.layout.compact;
  const sep = style.separator ?? '\u2013';
  const flagSize: readonly [number, number] = style.smallFlags ? [16, 11] : [FLAG_W, FLAG_H];
  const extraText = style.extraSides && style.extraSides > 0 ? `+${style.extraSides}` : null;
  let pillW = 0;
  if (extraText) {
    ctx.save();
    ctx.font = `500 ${theme.fontSize.small}px ${theme.font}`;
    pillW = Math.ceil(textWidth(ctx, extraText)) + 10;
    ctx.restore();
  }
  const contentRight = extraText ? right - pillW - 10 : right;
  const half = x + (contentRight - x) * 0.54;
  x = drawSide(ctx, sides[0], x, Math.min(contentRight, half), mid, input, names, half, flagSize) + 2;
  if (x + 18 > contentRight) return;
  ctx.save();
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  ctx.font = `400 ${theme.fontSize.small}px ${theme.font}`;
  ctx.fillStyle = theme.muted;
  ctx.fillText(sep, x, mid);
  const sepW = textWidth(ctx, sep);
  ctx.restore();
  const afterSecond = drawSide(ctx, sides[1], x + sepW + 6, contentRight, mid, input, names, contentRight, flagSize);
  if (extraText && right - pillW >= x + sepW + 6) {
    ctx.save();
    const px = Math.min(afterSecond, right - pillW);
    const ph = 15;
    const py = Math.round(mid - ph / 2);
    ctx.globalAlpha = 0.55;
    ctx.fillStyle = theme.hairline ?? theme.muted;
    ctx.beginPath();
    ctx.roundRect(px, py, pillW, ph, 7.5);
    ctx.fill();
    ctx.globalAlpha = 1;
    ctx.font = `500 ${theme.fontSize.small}px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(extraText, px + pillW / 2, mid + 0.5);
    ctx.restore();
  }
}

function drawOutcome(ctx: Canvas2D, lane: TimelineLane, mid: number, input: TimelinePaintInput): void {
  const { outcome } = lane;
  const { layout, theme, artCache } = input;
  if (!outcome || layout.compact) return;
  let x = layout.plotRight + OUTCOME_GAP;
  const right = layout.width - LABEL_PAD;
  if (right - x < 40) return;
  ctx.save();
  if (outcome.mark === 'flag') {
    drawMediaSlot(ctx, x, mid, MARK, MARK, { kind: 'art', art: outcome.flag ?? NO_ART, resolvedName: outcome.text }, theme, { artCache });
    x += MARK + 6;
  } else if (outcome.mark === 'bar') {
    ctx.fillStyle = theme.text;
    ctx.globalAlpha = 0.78;
    ctx.fillRect(Math.round(x + 2), Math.round(mid - 6), 2, 12);
    ctx.globalAlpha = 1;
    x += 12;
  } else {
    ctx.fillStyle = outcome.mark === 'ongoing' ? theme.accent : theme.muted;
    ctx.beginPath();
    ctx.arc(x + 4, mid, 3.5, 0, Math.PI * 2);
    ctx.fill();
    x += MARK + 6;
  }
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  ctx.font = `400 ${theme.fontSize.body}px ${theme.font}`;
  ctx.fillStyle = theme.text;
  const text = fitLine(outcome.text, Math.max(0, right - x), s => textWidth(ctx, s)).text;
  ctx.fillText(text, x, mid);
  x += textWidth(ctx, text) + 8;
  if (outcome.detail && right - x > 24) {
    ctx.font = `400 ${theme.fontSize.small}px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.fillText(fitLine(outcome.detail, right - x, s => textWidth(ctx, s)).text, x, mid);
  }
  ctx.restore();
}

function drawSplit(ctx: Canvas2D, span: TimelineSpan, x0: number, x1: number, mid: number, input: TimelinePaintInput): void {
  const { theme } = input;
  const [top, bottom] = span.halves!;
  if (x1 - x0 < 3) {
    const cx = (x0 + x1) / 2;
    const r = DIAMOND / 2;
    for (const [fill, upper] of [[top, true], [bottom, false]] as const) {
      ctx.save();
      ctx.fillStyle = fill;
      ctx.beginPath();
      ctx.moveTo(cx - r, mid);
      ctx.lineTo(cx, upper ? mid - r : mid + r);
      ctx.lineTo(cx + r, mid);
      ctx.closePath();
      ctx.fill();
      ctx.restore();
    }
    return;
  }
  for (const [fill, y] of [[top, Math.round(mid - HALF - 0.5)], [bottom, Math.round(mid + 0.5)]] as const) {
    drawTimelineBar(ctx, { x: x0, y, width: Math.max(1.5, x1 - x0), height: HALF, fill, theme, open: true, heldBefore: false, startBy: span.startBy, endUnseen: span.endUnseen });
  }
  if (span.endCap && !span.open) {
    ctx.save();
    ctx.fillStyle = theme.text;
    ctx.globalAlpha = 0.82;
    ctx.fillRect(Math.round(x1) + 3, Math.round(mid - HALF - 1), 1.5, HALF * 2 + 2);
    ctx.restore();
  }
}

function drawMarks(ctx: Canvas2D, span: TimelineSpan, mid: number, input: TimelinePaintInput): void {
  const { layout, theme, artCache } = input;
  for (const mark of span.marks ?? []) {
    if (mark.hour < layout.startHour || mark.hour > layout.endHour) continue;
    const x = hourToX(layout, mark.hour);
    drawMediaSlot(ctx, x - MARK / 2, mid, MARK, MARK, { kind: 'art', art: mark.art, resolvedName: mark.label }, theme, { artCache });
  }
}

function drawGroupRule(ctx: Canvas2D, lane: TimelineLane, mid: number, input: TimelinePaintInput): void {
  const { layout, theme } = input;
  const painted = paintedSpans(input.resolution, lane, layout);
  for (const { span, x0, x1 } of painted) {
    drawTimelineBar(ctx, {
      x: x0,
      y: Math.round(mid - GROUP_RULE_HEIGHT / 2),
      width: Math.max(2, x1 - x0),
      height: GROUP_RULE_HEIGHT,
      fill: lane.ruleColor ?? span.color,
      theme,
      open: span.open,
      endUnseen: span.endUnseen,
    });
  }
  const last = painted.at(-1);
  if (lane.endText) {
    const x = (last ? last.x1 : layout.plotLeft) + 6;
    if (layout.plotRight - x > 24) {
      ctx.save();
      ctx.font = `400 ${theme.fontSize.small}px ${theme.font}`;
      ctx.fillStyle = theme.muted;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillText(fitLine(lane.endText, layout.plotRight - x, s => textWidth(ctx, s)).text, x, mid);
      ctx.restore();
    }
  }
}

function laneHasIcons(lane: TimelineLane): boolean {
  return lane.spans.some(span => span.icon !== undefined);
}

function drawOpenedMarker(ctx: Canvas2D, lane: TimelineLane, mid: number, input: TimelinePaintInput): void {
  const marker = lane.openedMarker;
  if (!marker) return;
  const { layout, theme } = input;
  if (marker.hour <= layout.startHour || marker.hour > layout.endHour) return;
  const x = hourToX(layout, marker.hour);
  ctx.save();
  ctx.font = `400 ${theme.fontSize.small}px ${theme.font}`;
  ctx.fillStyle = theme.muted;
  ctx.textBaseline = 'middle';
  const w = textWidth(ctx, marker.label);
  if (x - 4 - w >= layout.plotLeft) {
    ctx.textAlign = 'right';
    ctx.fillText(marker.label, x - 4, mid);
  } else {
    const firstSpan = lane.spans[0];
    const barLeft = firstSpan ? hourToX(layout, firstSpan.firstHour) : layout.plotRight;
    if (x + 4 + w <= Math.min(barLeft - 4, layout.plotRight)) {
      ctx.textAlign = 'left';
      ctx.fillText(marker.label, x + 4, mid);
    }
  }
  ctx.restore();
}

function drawHeader(ctx: Canvas2D, input: TimelinePaintInput): void {
  const { model, layout, theme, artCache } = input;
  const header = model.header;
  if (!header || layout.headerHeight <= 0) return;
  const mid = layout.headerHeight / 2;
  let x = LABEL_PAD;
  if (header.portrait) {
    drawMediaSlot(
      ctx,
      x,
      mid,
      34,
      44,
      { kind: 'art', art: header.portrait, ...(header.portraitName ? { name: header.portraitName } : {}) },
      theme,
      { artCache, radius: 4 },
    );
    x += 44;
  }
  const showFullKpis = layout.width >= 560 && !!header.kpis?.length;
  let rightEdge = layout.width - LABEL_PAD;
  if (showFullKpis) {
    ctx.save();
    ctx.textAlign = 'right';
    ctx.textBaseline = 'middle';
    const kpis = header.kpis!;
    let kx = rightEdge;
    for (let i = kpis.length - 1; i >= 0; i--) {
      const kpi = kpis[i]!;
      ctx.font = `600 ${theme.fontSize.body + 1}px ${theme.font}`;
      const valW = textWidth(ctx, kpi.value);
      ctx.font = `400 ${theme.fontSize.small}px ${theme.font}`;
      const labW = textWidth(ctx, kpi.label);
      const colW = Math.max(valW, labW);
      if (kx - colW < x + 120) break;
      ctx.font = `600 ${theme.fontSize.body + 1}px ${theme.font}`;
      ctx.fillStyle = theme.text;
      ctx.fillText(kpi.value, kx, mid - 8);
      ctx.font = `400 ${theme.fontSize.small}px ${theme.font}`;
      ctx.fillStyle = theme.muted;
      ctx.fillText(kpi.label, kx, mid + 9);
      kx -= colW + 16;
    }
    rightEdge = kx - 8;
    ctx.restore();
  }
  ctx.save();
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  ctx.font = `600 ${theme.fontSize.body + 1}px ${theme.font}`;
  ctx.fillStyle = theme.text;
  ctx.fillText(fitLine(header.title, Math.max(40, rightEdge - x), s => textWidth(ctx, s)).text, x, header.subtitle ? mid - 8 : mid);
  if (header.subtitle || (!showFullKpis && header.compactKpi)) {
    let sx = x;
    const sy = mid + 9;
    let subRight = rightEdge;
    if (!showFullKpis && header.compactKpi) {
      ctx.font = `500 ${theme.fontSize.small}px ${theme.font}`;
      ctx.fillStyle = theme.text;
      const cw = textWidth(ctx, header.compactKpi);
      if (layout.width - LABEL_PAD - cw > sx + 70) {
        ctx.textAlign = 'right';
        ctx.fillText(header.compactKpi, layout.width - LABEL_PAD, sy);
        ctx.textAlign = 'left';
        subRight = layout.width - LABEL_PAD - cw - 10;
      }
    }
    if (header.flag && sx + 18 <= subRight) {
      drawMediaSlot(ctx, sx, sy, 15, 10, { kind: 'flag', art: header.flag, ...(header.flagTag ? { tag: header.flagTag } : {}) }, theme, { artCache });
      sx += 20;
    }
    if (header.subtitle && subRight - sx > 20) {
      ctx.font = `400 ${theme.fontSize.small}px ${theme.font}`;
      ctx.fillStyle = theme.muted;
      ctx.fillText(fitLine(header.subtitle, subRight - sx, s => textWidth(ctx, s)).text, sx, sy);
    }
  }
  ctx.restore();
}

function drawTopAxis(ctx: Canvas2D, ticks: readonly TimeAxisTick[], input: TimelinePaintInput): void {
  const { layout, theme } = input;
  ctx.save();
  ctx.font = `400 ${theme.fontSize.small}px ${theme.font}`;
  ctx.fillStyle = theme.muted;
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  const y = layout.axisTop + TOP_AXIS_HEIGHT / 2;
  let lastRight = -Infinity;
  for (const tick of ticks) {
    const x = hourToX(layout, tick.hour);
    if (x < layout.plotLeft - 2 || x > layout.plotRight - 14) continue;
    const w = textWidth(ctx, tick.label);
    if (x < lastRight + 8) continue;
    ctx.fillText(tick.label, x, y);
    lastRight = x + w;
  }
  ctx.restore();
}

function drawMeters(ctx: Canvas2D, input: TimelinePaintInput): void {
  const { model, layout, theme } = input;
  const meters = model.meters;
  if (!meters || layout.metersHeight <= 0) return;
  ctx.save();
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  ctx.font = `600 ${theme.fontSize.small}px ${theme.font}`;
  ctx.fillStyle = theme.text;
  ctx.fillText(meters.title, LABEL_PAD, layout.metersTop + 10);
  if (!meters.items.length) {
    ctx.font = `400 ${theme.fontSize.small}px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.fillText(meters.empty ?? 'None recorded', LABEL_PAD, layout.metersTop + METERS_HEADER_HEIGHT + 10);
    ctx.restore();
    return;
  }
  const cols = METERS_PER_ROW;
  const gap = 16;
  const colW = Math.max(80, (layout.width - LABEL_PAD * 2 - gap * (cols - 1)) / cols);
  const capacity = cols * Math.max(1, Math.floor((layout.metersHeight - METERS_HEADER_HEIGHT - 8) / METERS_ROW_HEIGHT));
  const overflow = meters.items.length > capacity ? meters.items.length - capacity + 1 : 0;
  if (overflow) {
    const index = capacity - 1;
    ctx.font = `400 ${theme.fontSize.small}px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.fillText(`+${overflow} more`, LABEL_PAD + (index % cols) * (colW + gap), layout.metersTop + METERS_HEADER_HEIGHT + Math.floor(index / cols) * METERS_ROW_HEIGHT + 6);
  }
  meters.items.slice(0, overflow ? capacity - 1 : capacity).forEach((item, index) => {
    const col = index % cols;
    const row = Math.floor(index / cols);
    const x = LABEL_PAD + col * (colW + gap);
    const y = layout.metersTop + METERS_HEADER_HEIGHT + row * METERS_ROW_HEIGHT;
    ctx.font = `400 ${theme.fontSize.small}px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.textAlign = 'right';
    const figW = textWidth(ctx, item.figure);
    ctx.fillText(item.figure, x + colW, y + 6);
    ctx.textAlign = 'left';
    ctx.font = `500 ${theme.fontSize.small}px ${theme.font}`;
    ctx.fillStyle = theme.text;
    ctx.fillText(fitLine(item.label, Math.max(24, colW - figW - 8), s => textWidth(ctx, s)).text, x, y + 6);
    const barY = y + 17;
    const barH = 5;
    ctx.globalAlpha = 0.45;
    ctx.fillStyle = theme.hairline ?? theme.muted;
    ctx.beginPath();
    ctx.roundRect(x, barY, colW, barH, 2.5);
    ctx.fill();
    ctx.globalAlpha = 1;
    if (item.fraction !== null && item.fraction > 0) {
      ctx.fillStyle = theme.accent;
      ctx.beginPath();
      ctx.roundRect(x, barY, Math.max(4, Math.round(colW * Math.min(1, item.fraction))), barH, 2.5);
      ctx.fill();
    }
  });
  ctx.restore();
}

function drawBars(ctx: Canvas2D, lane: TimelineLane, mid: number, input: TimelinePaintInput): void {
  const { layout, model, theme, artCache } = input;
  const icons = model.iconsOnBars && laneHasIcons(lane);
  const barH = lane.barHeight ?? (icons ? BAR_HEIGHT_ICONS : BAR_HEIGHT);
  const y = Math.round(mid - barH / 2);
  const painted = paintedSpans(input.resolution, lane, layout);
  const hasHistogram = lane.spans.some(span => span.histogram !== undefined);
  if (hasHistogram) {
    ctx.save();
    ctx.strokeStyle = theme.hairline ?? theme.muted;
    ctx.globalAlpha = 0.45;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(layout.plotLeft, Math.round(mid) + 0.5);
    ctx.lineTo(layout.plotRight, Math.round(mid) + 0.5);
    ctx.stroke();
    ctx.restore();
  }
  const diamondItems: { readonly cx: number; readonly color: string; readonly text?: string }[] = [];
  for (const { span, x0, x1 } of painted) {
    if (span.histogram) {
      const { positive, negative, max } = span.histogram;
      const halfMax = Math.max(4, Math.floor(barH / 2) - 1);
      const w = Math.max(2, x1 - x0 - 1);
      ctx.save();
      if (positive > 0 && max > 0) {
        const h = Math.max(2, Math.round((positive / max) * halfMax));
        ctx.fillStyle = theme.green;
        ctx.fillRect(x0, Math.round(mid) - h, w, h);
      }
      if (negative > 0 && max > 0) {
        const h = Math.max(2, Math.round((negative / max) * halfMax));
        ctx.fillStyle = theme.red;
        ctx.fillRect(x0, Math.round(mid) + 1, w, h);
      }
      ctx.restore();
      continue;
    }
    if (span.diamond) {
      const cx = Math.max(layout.plotLeft + 4, Math.min(layout.plotRight - 4, x0));
      diamondItems.push({ cx, color: span.diamond.color, ...(span.diamond.text ? { text: span.diamond.text } : {}) });
      continue;
    }
    if (span.halves) {
      drawSplit(ctx, span, x0, x1, mid, input);
      drawMarks(ctx, span, mid, input);
      continue;
    }
    const barW = Math.max(1.5, x1 - x0 - (span.open ? 0 : 1));
    drawTimelineBar(ctx, {
      x: x0,
      y,
      width: barW,
      height: barH,
      fill: span.color,
      theme,
      open: span.open,
      heldBefore: span.heldBefore,
      startBy: span.startBy,
      endUnseen: span.endUnseen,
      ...(icons && span.icon ? { icon: toArtSlotState(resolveCachedArt(span.icon, artCache), span.label ?? lane.label) } : {}),
      ...(span.label && barH >= BAR_HEIGHT && barW >= 44 ? { label: span.label } : {}),
    });
    drawMarks(ctx, span, mid, input);
  }
  if (diamondItems.length) {
    const r = 4.5;
    ctx.save();
    for (const item of diamondItems) {
      ctx.fillStyle = item.color;
      ctx.beginPath();
      ctx.moveTo(item.cx, mid - r);
      ctx.lineTo(item.cx + r, mid);
      ctx.lineTo(item.cx, mid + r);
      ctx.lineTo(item.cx - r, mid);
      ctx.closePath();
      ctx.fill();
    }
    if (layout.width >= 560) {
      ctx.font = `400 ${theme.fontSize.small}px ${theme.font}`;
      ctx.fillStyle = theme.muted;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      let lastLabelRight = -Infinity;
      for (let i = 0; i < diamondItems.length; i++) {
        const item = diamondItems[i]!;
        if (!item.text) continue;
        const tx = item.cx + r + 5;
        let nextX = layout.plotRight - 2;
        for (let j = i + 1; j < diamondItems.length; j++) {
          if (diamondItems[j]!.cx > item.cx + 1) {
            nextX = diamondItems[j]!.cx - r - 6;
            break;
          }
        }
        const avail = nextX - tx;
        if (tx < lastLabelRight || avail < 36) continue;
        const fitted = fitLine(item.text, avail, s => textWidth(ctx, s)).text;
        ctx.fillText(fitted, tx, mid);
        lastLabelRight = tx + textWidth(ctx, fitted) + 10;
      }
    }
    ctx.restore();
  }
}

export function drawTimeline(ctx: Canvas2D, input: TimelinePaintInput): readonly TimeAxisTick[] {
  const { model, layout, theme, scrollTop } = input;
  ctx.clearRect(0, 0, layout.width, layout.height);
  ctx.fillStyle = theme.surface;
  ctx.fillRect(0, 0, layout.width, layout.height);
  const ticks = computeTimeAxisTicks(layout.startHour, layout.endHour, layout.plotRight - layout.plotLeft);

  drawHeader(ctx, input);

  if (model.legend?.length && layout.bodyTop > layout.headerHeight) {
    drawLegendStrip(ctx, 0, layout.headerHeight, layout.width, LEGEND_HEIGHT, { entries: model.legend, padX: LABEL_PAD }, theme);
  }

  if (layout.axisPosition === 'top') {
    drawTopAxis(ctx, ticks, input);
  }

  const bodyBottom = layout.bodyTop + layout.bodyHeight;
  ctx.save();
  ctx.beginPath();
  ctx.rect(0, layout.bodyTop, layout.width, layout.bodyHeight);
  ctx.clip();
  drawTimeAxisGrid(ctx, ticks, layout, layout.bodyTop, bodyBottom, theme);
  for (const gap of coalescedGaps(input.resolution)) {
    const x0 = Math.max(layout.plotLeft, hourToX(layout, gap.firstHour));
    const x1 = gap.lastHour >= model.hour ? layout.plotRight : Math.min(layout.plotRight, hourToX(layout, gap.lastHour + 1));
    if (x1 > x0) drawGapBand(ctx, x0, layout.bodyTop, x1 - x0, layout.bodyHeight, theme);
  }
  const [first, end] = visibleRange(layout, scrollTop);
  for (let index = first; index < end; index++) {
    const placed = layout.lanes[index]!;
    const y = layout.bodyTop + placed.y - scrollTop;
    const { lane } = placed;
    if (lane.kind === 'group' && placed.ordinal > 0) drawRowHairline(ctx, LABEL_PAD, layout.width - LABEL_PAD, y, theme, 'strong');
    if (input.selectedKey === lane.key) drawRowSelected(ctx, 0, y, layout.width, placed.height, theme);
    else if (input.inspectedKey === lane.key) drawRowHover(ctx, 0, y, layout.width, placed.height, theme);
    if (input.focusedKey === lane.key) drawRowFocusRing(ctx, 0, y, layout.width, placed.height, theme);
    drawLabel(ctx, placed, y, input);
    const mid = y + placed.height / 2;
    if (lane.kind === 'group') drawGroupRule(ctx, lane, mid, input);
    else if (lane.kind === 'item') {
      drawOpenedMarker(ctx, lane, mid, input);
      drawBars(ctx, lane, mid, input);
    }
    drawOutcome(ctx, lane, mid, input);
  }
  if (layout.axisPosition === 'top') {
    ctx.save();
    ctx.strokeStyle = theme.accent;
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.moveTo(layout.plotRight - 0.5, layout.bodyTop);
    ctx.lineTo(layout.plotRight - 0.5, Math.min(bodyBottom, layout.bodyTop + layout.contentHeight));
    ctx.stroke();
    ctx.restore();
  }
  if (scrollTop + layout.bodyHeight < layout.contentHeight - 1) {
    drawScrollFade(ctx, 0, bodyBottom - 24, layout.width, 24, theme, 'bottom');
  }
  ctx.restore();

  if (layout.axisPosition === 'bottom') {
    drawTimeAxis(ctx, ticks, {
      plotLeft: layout.plotLeft,
      plotRight: layout.plotRight,
      startHour: layout.startHour,
      endHour: layout.endHour,
      axisTop: layout.axisTop,
      playhead: layout.endHour,
      playheadLabel: formatDateOnly(layout.endHour),
      theme,
    });
  }

  drawMeters(ctx, input);
  return ticks;
}

export function hitTimeline(layout: TimelineLayout, resolution: TimelineResolution, x: number, y: number, scrollTop: number): TimelineHit | null {
  if (y < layout.bodyTop || y >= layout.bodyTop + layout.bodyHeight) return null;
  const placed = placedLaneAt(layout, y - layout.bodyTop + scrollTop);
  if (!placed) return null;
  const { lane } = placed;
  if (lane.kind === 'heading') return null;
  if (lane.expandable && x < layout.labelWidth) return { kind: 'toggle', laneKey: lane.key };
  if (x < layout.plotLeft || x > layout.plotRight) return { kind: 'lane', laneKey: lane.key };
  const hour = Math.floor(xToHour(layout, x));
  let nearest: { readonly span: TimelineSpan; readonly distance: number } | null = null;
  for (const { span, x0, x1 } of paintedSpans(resolution, lane, layout)) {
    const direct = x >= x0 && x < x1;
    const distance = direct ? -1 : x < x0 ? x0 - x : x - x1;
    if (distance <= HIT_SLACK && (!nearest || distance < nearest.distance)) nearest = { span, distance };
    if (direct) break;
  }
  if (!nearest) return { kind: 'lane', laneKey: lane.key, hour };
  const span = spanUnder(resolution, nearest.span, hour);
  return { kind: 'span', laneKey: lane.key, spanId: span.id, hour: Math.max(span.firstHour, span.open ? hour : Math.min(hour, span.lastHour)) };
}
