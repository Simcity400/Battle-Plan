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
  type ArtSlotState,
  type TimeAxisTick,
} from '../../draw/kit.ts';
import { drawMediaSlot, toArtSlotState } from '../art/adapters.ts';
import { unavailableArt } from '../art/types.ts';
import type { ArtHandle, BoundedReadView, FamilyTheme } from '../contract.ts';
import {
  BAR_HEIGHT,
  BAR_HEIGHT_ICONS,
  GROUP_RULE_HEIGHT,
  hourToX,
  LANE_ICON,
  placedLaneAt,
  visibleRange,
  xToHour,
  type PlacedLane,
  type TimelineLayout,
} from './layout.ts';
import { laneOpen, type TimelineHit, type TimelineLane, type TimelineModel, type TimelineSideLabel, type TimelineSpan } from './model.ts';
import { coalescedGaps, paintedSpans, type TimelineResolution } from './resolution.ts';

export interface TimelinePaintInput {
  readonly model: TimelineModel;
  readonly layout: TimelineLayout;
  readonly readView: BoundedReadView;
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

function artState(readView: BoundedReadView, handle: ArtHandle | null | undefined, missing: boolean | undefined, name: string): ArtSlotState {
  if (handle) return toArtSlotState(readView.art(handle), name);
  return missing ? { kind: 'missing', resolvedName: name } : { kind: 'none' };
}

function drawLabel(ctx: Canvas2D, placed: PlacedLane, y: number, input: TimelinePaintInput): void {
  const { lane } = placed;
  const { layout, theme, readView, expanded } = input;
  const mid = y + placed.height / 2;
  let x = LABEL_PAD + (lane.depth ?? 0) * INDENT;
  const right = layout.labelWidth;
  if (lane.expandable) {
    drawChevron(ctx, x - 4, mid, laneOpen(lane, expanded), theme);
    x += CHEVRON + 6;
  }
  if (lane.flag || lane.flagTag) {
    drawMediaSlot(ctx, x, mid, FLAG_W, FLAG_H, {
      kind: 'flag',
      art: lane.flag ? readView.art(lane.flag) : unavailableArt('Flag unavailable'),
      tag: lane.flagTag,
    }, theme);
    x += FLAG_W + 8;
  } else if (lane.icon || lane.iconMissing) {
    drawMediaSlot(ctx, x, mid, LANE_ICON, LANE_ICON, {
      kind: 'art',
      art: lane.icon ? readView.art(lane.icon) : unavailableArt('Art unavailable'),
      resolvedName: lane.label,
    }, theme, { radius: 3 });
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
    drawSides(ctx, lane.sides, x, right, lane.subtitle && !layout.compact ? mid - 7 : mid, input);
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

function drawSide(ctx: Canvas2D, side: TimelineSideLabel, x: number, right: number, mid: number, input: TimelinePaintInput, leadName: boolean, stripLimit = right): number {
  const { theme, readView } = input;
  ctx.save();
  if (side.dim) ctx.globalAlpha = LOSER_ALPHA;
  if (x + FLAG_W <= right) {
    drawMediaSlot(ctx, x, mid, FLAG_W, FLAG_H, { kind: 'flag', art: side.flag ? readView.art(side.flag) : unavailableArt('Flag unavailable'), tag: side.flagTag }, theme);
    x += FLAG_W + 6;
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
    drawMediaSlot(ctx, x, mid, STRIP_W, STRIP_H, { kind: 'flag', art: member.flag ? readView.art(member.flag) : unavailableArt('Flag unavailable'), tag: member.tag }, theme);
    x += STRIP_W + 2;
  }
  ctx.restore();
  return x;
}

function drawSides(ctx: Canvas2D, sides: readonly [TimelineSideLabel, TimelineSideLabel], x: number, right: number, mid: number, input: TimelinePaintInput): void {
  const { theme } = input;
  const names = !input.layout.compact;
  const half = x + (right - x) * 0.5;
  x = drawSide(ctx, sides[0], x, right, mid, input, names, half) + 4;
  if (x + 20 > right) return;
  ctx.save();
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  ctx.font = `400 ${theme.fontSize.body}px ${theme.font}`;
  ctx.fillStyle = theme.muted;
  ctx.fillText('\u2013', x, mid);
  ctx.restore();
  drawSide(ctx, sides[1], x + 14, right, mid, input, names);
}

function drawOutcome(ctx: Canvas2D, lane: TimelineLane, mid: number, input: TimelinePaintInput): void {
  const { outcome } = lane;
  const { layout, theme, readView } = input;
  if (!outcome || layout.compact) return;
  let x = layout.plotRight + OUTCOME_GAP;
  const right = layout.width - LABEL_PAD;
  if (right - x < 40) return;
  ctx.save();
  if (outcome.mark === 'flag') {
    drawMediaSlot(ctx, x, mid, MARK, MARK, { kind: 'art', art: outcome.flag ? readView.art(outcome.flag) : unavailableArt('Art unavailable'), resolvedName: outcome.text }, theme);
  } else {
    ctx.fillStyle = outcome.mark === 'ongoing' ? theme.accent : theme.muted;
    ctx.beginPath();
    ctx.arc(x + 4, mid, 3.5, 0, Math.PI * 2);
    ctx.fill();
  }
  x += MARK + 6;
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
}

function drawMarks(ctx: Canvas2D, span: TimelineSpan, mid: number, input: TimelinePaintInput): void {
  const { layout, theme, readView } = input;
  for (const mark of span.marks ?? []) {
    if (mark.hour < layout.startHour || mark.hour > layout.endHour) continue;
    const x = hourToX(layout, mark.hour);
    drawMediaSlot(ctx, x - MARK / 2, mid, MARK, MARK, { kind: 'art', art: mark.art ? readView.art(mark.art) : unavailableArt('Art unavailable'), resolvedName: mark.label }, theme);
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
  return lane.spans.some(span => span.icon || span.iconMissing);
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

function drawBars(ctx: Canvas2D, lane: TimelineLane, mid: number, input: TimelinePaintInput): void {
  const { layout, model, theme, readView } = input;
  const icons = model.iconsOnBars && laneHasIcons(lane);
  const barH = lane.barHeight ?? (icons ? BAR_HEIGHT_ICONS : BAR_HEIGHT);
  const y = Math.round(mid - barH / 2);
  for (const { span, x0, x1 } of paintedSpans(input.resolution, lane, layout)) {
    if (span.halves) {
      drawSplit(ctx, span, x0, x1, mid, input);
      drawMarks(ctx, span, mid, input);
      continue;
    }
    drawTimelineBar(ctx, {
      x: x0,
      y,
      width: Math.max(1.5, x1 - x0 - (span.open ? 0 : 1)),
      height: barH,
      fill: span.color,
      theme,
      open: span.open,
      heldBefore: span.heldBefore,
      startBy: span.startBy,
      endUnseen: span.endUnseen,
      ...(icons && (span.icon || span.iconMissing) ? { icon: artState(readView, span.icon, span.iconMissing, span.label ?? lane.label) } : {}),
      ...(span.label && barH >= BAR_HEIGHT ? { label: span.label } : {}),
    });
    drawMarks(ctx, span, mid, input);
  }
}

export function drawTimeline(ctx: Canvas2D, input: TimelinePaintInput): readonly TimeAxisTick[] {
  const { model, layout, theme, scrollTop } = input;
  ctx.clearRect(0, 0, layout.width, layout.height);
  ctx.fillStyle = theme.surface;
  ctx.fillRect(0, 0, layout.width, layout.height);
  const ticks = computeTimeAxisTicks(layout.startHour, layout.endHour, layout.plotRight - layout.plotLeft);

  if (model.legend?.length && layout.bodyTop > 0) {
    drawLegendStrip(ctx, 0, 0, layout.width, layout.bodyTop, { entries: model.legend, padX: LABEL_PAD }, theme);
  }

  ctx.save();
  ctx.beginPath();
  ctx.rect(0, layout.bodyTop, layout.width, layout.bodyHeight);
  ctx.clip();
  drawTimeAxisGrid(ctx, ticks, layout, layout.bodyTop, layout.axisTop, theme);
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
  if (scrollTop + layout.bodyHeight < layout.contentHeight - 1) {
    drawScrollFade(ctx, 0, layout.axisTop - 24, layout.width, 24, theme, 'bottom');
  }
  ctx.restore();

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
  return ticks;
}

export function hitTimeline(layout: TimelineLayout, resolution: TimelineResolution, x: number, y: number, scrollTop: number): TimelineHit | null {
  if (y < layout.bodyTop || y >= layout.axisTop) return null;
  const placed = placedLaneAt(layout, y - layout.bodyTop + scrollTop);
  if (!placed) return null;
  const { lane } = placed;
  if (lane.kind === 'heading') return null;
  if (lane.expandable && x < layout.labelWidth) return { kind: 'toggle', laneKey: lane.key };
  if (x < layout.plotLeft || x > layout.plotRight) return { kind: 'lane', laneKey: lane.key };
  const hour = Math.floor(xToHour(layout, x));
  let nearest: { readonly spanId: string; readonly distance: number } | null = null;
  for (const { span, x0, x1 } of paintedSpans(resolution, lane, layout)) {
    if (x >= x0 && x < x1) return { kind: 'span', laneKey: lane.key, spanId: span.id, hour };
    const distance = x < x0 ? x0 - x : x - x1;
    if (distance <= HIT_SLACK && (!nearest || distance < nearest.distance)) nearest = { spanId: span.id, distance };
  }
  return nearest ? { kind: 'span', laneKey: lane.key, spanId: nearest.spanId, hour } : { kind: 'lane', laneKey: lane.key, hour };
}

const SEP = '\u001f';

export function encodeTimelineHit(hit: TimelineHit | null): string | null {
  if (!hit) return null;
  return `${hit.kind}:${hit.laneKey}${SEP}${hit.spanId ?? ''}${SEP}${hit.hour ?? ''}`;
}

export function decodeTimelineHit(token: string | null | undefined): TimelineHit | null {
  if (!token) return null;
  const colon = token.indexOf(':');
  if (colon < 0) return null;
  const kind = token.slice(0, colon);
  if (kind !== 'lane' && kind !== 'span' && kind !== 'toggle') return null;
  const [laneKey, spanId, hourText] = token.slice(colon + 1).split(SEP);
  if (!laneKey) return null;
  const hour = hourText ? Number(hourText) : NaN;
  return {
    kind,
    laneKey,
    ...(spanId ? { spanId } : {}),
    ...(Number.isFinite(hour) ? { hour } : {}),
  };
}
