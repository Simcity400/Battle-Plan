import { fitLine } from '../../charts/labels.ts';
import { textWidth, type Canvas2D } from '../../charts/textWidth.ts';
import { formatDateOnly } from '../valueFormat.ts';
import {
  computeTimeAxisTicks,
  drawArtSlot,
  drawChevron,
  drawFlagSlot,
  drawGapBand,
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
import { toArtSlotState, toFlagSlotState } from '../art/adapters.ts';
import type { ArtHandle, BoundedReadView, FamilyTheme } from '../contract.ts';
import {
  BAR_HEIGHT,
  BAR_HEIGHT_ICONS,
  GROUP_RULE_HEIGHT,
  hourToX,
  LANE_ICON,
  placedLaneAt,
  xToHour,
  type PlacedLane,
  type TimelineLayout,
} from './layout.ts';
import { laneOpen, type TimelineHit, type TimelineLane, type TimelineModel } from './model.ts';

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
}

const LABEL_PAD = 8;
const CHEVRON = 10;
const INDENT = 18;
const FLAG_W = 20;
const FLAG_H = 13;

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
    drawChevron(ctx, x, mid, laneOpen(lane, expanded), theme.muted, CHEVRON);
    x += CHEVRON + 6;
  }
  if (lane.flag || lane.flagTag) {
    const state = lane.flag ? toFlagSlotState(readView.art(lane.flag), lane.flagTag) : { kind: 'missing' as const, tag: lane.flagTag };
    drawFlagSlot(ctx, x, Math.round(mid - FLAG_H / 2), state, theme, { width: FLAG_W, height: FLAG_H });
    x += FLAG_W + 8;
  } else if (lane.icon || lane.iconMissing) {
    const state = artState(readView, lane.icon, lane.iconMissing, lane.label);
    drawArtSlot(ctx, x, Math.round(mid - LANE_ICON / 2), LANE_ICON, LANE_ICON, state, theme, { radius: 3 });
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
  ctx.fillText(fitLine(lane.label, width, s => textWidth(ctx, s)).text, x, twoLines ? mid - 7 : mid);
  if (twoLines) {
    ctx.font = `400 ${theme.fontSize.small}px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.fillText(fitLine(lane.subtitle!, width, s => textWidth(ctx, s)).text, x, mid + 8);
  }
  ctx.restore();
}

function drawGroupRule(ctx: Canvas2D, lane: TimelineLane, mid: number, layout: TimelineLayout, theme: FamilyTheme): void {
  for (const span of lane.spans) {
    const x0 = Math.max(layout.plotLeft, hourToX(layout, span.firstHour));
    const x1 = Math.min(layout.plotRight, hourToX(layout, span.lastHour));
    if (x1 < x0) continue;
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
  if (lane.endText) {
    const last = lane.spans.at(-1);
    const x = last ? Math.min(layout.plotRight, hourToX(layout, last.lastHour)) + 6 : layout.plotLeft;
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

function drawBars(ctx: Canvas2D, lane: TimelineLane, mid: number, input: TimelinePaintInput): void {
  const { layout, model, theme, readView } = input;
  const icons = model.iconsOnBars && laneHasIcons(lane);
  const barH = icons ? BAR_HEIGHT_ICONS : BAR_HEIGHT;
  const y = Math.round(mid - barH / 2);
  for (const span of lane.spans) {
    const rawX0 = hourToX(layout, span.firstHour);
    const rawX1 = hourToX(layout, span.lastHour + 1);
    const x0 = Math.max(layout.plotLeft, rawX0);
    const x1 = Math.min(layout.plotRight, rawX1);
    if (x1 <= layout.plotLeft || x0 >= layout.plotRight) continue;
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
      ...(span.label ? { label: span.label } : {}),
    });
  }
}

export function drawTimeline(ctx: Canvas2D, input: TimelinePaintInput): readonly TimeAxisTick[] {
  const { model, layout, theme, scrollTop } = input;
  ctx.clearRect(0, 0, layout.width, layout.height);
  ctx.fillStyle = theme.surface;
  ctx.fillRect(0, 0, layout.width, layout.height);
  const ticks = computeTimeAxisTicks(layout.startHour, layout.endHour, layout.plotRight - layout.plotLeft);

  ctx.save();
  ctx.beginPath();
  ctx.rect(0, 0, layout.width, layout.bodyHeight);
  ctx.clip();
  drawTimeAxisGrid(ctx, ticks, layout, 0, layout.bodyHeight, theme);
  for (const gap of model.unobserved) {
    const x0 = Math.max(layout.plotLeft, hourToX(layout, gap.firstHour));
    const x1 = Math.min(layout.plotRight, hourToX(layout, gap.lastHour + 1));
    if (x1 > x0) drawGapBand(ctx, x0, 0, x1 - x0, layout.bodyHeight, theme);
  }
  for (const placed of layout.lanes) {
    const y = placed.y - scrollTop;
    if (y + placed.height <= 0 || y >= layout.bodyHeight) continue;
    const { lane } = placed;
    if (lane.kind === 'group' && placed.ordinal > 0) drawRowHairline(ctx, LABEL_PAD, layout.width - LABEL_PAD, y, theme, 'strong');
    if (input.selectedKey === lane.key) drawRowSelected(ctx, 0, y, layout.width, placed.height, theme);
    else if (input.inspectedKey === lane.key) drawRowHover(ctx, 0, y, layout.width, placed.height, theme);
    if (input.focusedKey === lane.key) drawRowFocusRing(ctx, 0, y, layout.width, placed.height, theme);
    drawLabel(ctx, placed, y, input);
    const mid = y + placed.height / 2;
    if (lane.kind === 'group') drawGroupRule(ctx, lane, mid, layout, theme);
    else if (lane.kind === 'item') drawBars(ctx, lane, mid, input);
  }
  if (scrollTop + layout.bodyHeight < layout.contentHeight - 1) {
    drawScrollFade(ctx, 0, layout.bodyHeight - 24, layout.width, 24, theme, 'bottom');
  }
  ctx.restore();

  drawTimeAxis(ctx, ticks, {
    plotLeft: layout.plotLeft,
    plotRight: layout.plotRight,
    startHour: layout.startHour,
    endHour: layout.endHour,
    axisTop: layout.axisTop,
    playhead: model.hour,
    playheadLabel: formatDateOnly(model.hour),
    theme,
  });
  return ticks;
}

export function hitTimeline(layout: TimelineLayout, x: number, y: number, scrollTop: number): TimelineHit | null {
  if (y < 0 || y >= layout.bodyHeight) return null;
  const placed = placedLaneAt(layout, y + scrollTop);
  if (!placed) return null;
  const { lane } = placed;
  if (lane.kind === 'heading') return null;
  if (lane.expandable && x < layout.labelWidth) return { kind: 'toggle', laneKey: lane.key };
  if (x < layout.plotLeft || x > layout.plotRight) return { kind: 'lane', laneKey: lane.key };
  const hour = Math.floor(xToHour(layout, x));
  const slack = (layout.endHour - layout.startHour) / Math.max(1, layout.plotRight - layout.plotLeft) * 3;
  for (const span of lane.spans) {
    if (hour >= span.firstHour - slack && hour <= span.lastHour + slack) {
      return { kind: 'span', laneKey: lane.key, spanId: span.id, hour: Math.min(Math.max(hour, span.firstHour), span.lastHour) };
    }
  }
  return { kind: 'lane', laneKey: lane.key, hour };
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
