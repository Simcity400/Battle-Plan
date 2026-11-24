import { LABEL_WEIGHT, VALUE_WEIGHT } from '../../charts/draw.ts';
import { ellipsise } from '../../charts/labels.ts';
import { textWidth, type Canvas2D } from '../../charts/textWidth.ts';
import {
  computeTimeAxisTicks,
  contrastInkForFill,
  drawCrosshairLine,
  drawFlagSlot,
  drawGapBand,
  drawLegendStrip,
  drawMissingMark,
  drawNameSlot,
  drawPlayheadLine,
  drawRowFocusRing,
  drawRowHairline,
  drawRowHover,
  drawRowSelected,
  drawTimeAxis,
  FLAG_SLOT_HEIGHT,
  FLAG_SLOT_WIDTH,
  resolveKitTokens,
} from '../../draw/kit.ts';
import { mix } from '../../shell/appearance/color.ts';
import { toFlagSlotState } from '../art/adapters.ts';
import { collectArtUrls } from '../art/provider.ts';
import type { ArtProvider, FamilyTheme } from '../contract.ts';
import type { IdentityIndex } from '../identity/index.ts';
import { drawHatchRect } from '../valueCanvas.ts';
import { formatDateOnly } from '../valueFormat.ts';
import {
  hourToLaneX,
  LANE_PAD,
  LANE_ROW_GAP,
  projectLaneRunsToPixels,
  type HeatLanesLayout,
} from './lanesLayout.ts';
import type { HeatLaneRow, HeatLanesModel } from './lanesModel.ts';
import { marginColour, splitStripes } from './scale.ts';

export interface DrawHeatLanesInput {
  readonly model: HeatLanesModel;
  readonly layout: HeatLanesLayout;
  readonly theme: FamilyTheme;
  readonly scrollTop: number;
  readonly hoveredRow: number | null;
  readonly hoveredHour: number | null;
  readonly selectedRowId: string | null;
  readonly focusedRowId: string | null;
  readonly artProvider?: ArtProvider;
  readonly identityIndex?: IdentityIndex | null;
}

export function collectVisibleLaneArt(
  model: HeatLanesModel,
  layout: HeatLanesLayout,
  identityIndex?: IdentityIndex | null,
): readonly string[] {
  const handles = [];
  for (let i = layout.firstRow; i < layout.endRow; i++) {
    const row = model.rows[i];
    if (row?.flag) handles.push(row.flag);
  }
  return collectArtUrls(handles, identityIndex);
}

function drawRowFlag(
  ctx: Canvas2D,
  row: HeatLaneRow,
  x: number,
  y: number,
  rowHeight: number,
  theme: FamilyTheme,
  artProvider?: ArtProvider,
): void {
  const fy = Math.round(y + (rowHeight - FLAG_SLOT_HEIGHT) / 2);
  if (row.flag) {
    const state = artProvider
      ? toFlagSlotState(artProvider.resolve(row.flag), row.tag ?? row.id)
      : ({ kind: 'pending' } as const);
    drawFlagSlot(ctx, x, fy, state, theme);
    return;
  }
  if (row.flagPending) {
    drawFlagSlot(ctx, x, fy, { kind: 'pending' }, theme);
    return;
  }
  drawFlagSlot(ctx, x, fy, { kind: 'missing', tag: row.tag ?? row.id }, theme);
}

export const READOUT_SWATCH = 10;
export const READOUT_VALUE_SLOT = 50;
const READOUT_GAP = 8;

function drawReadoutSwatch(ctx: Canvas2D, x: number, middleY: number, swatch: string, theme: FamilyTheme): void {
  ctx.save();
  ctx.beginPath();
  ctx.roundRect(x + 0.5, middleY - READOUT_SWATCH / 2 + 0.5, READOUT_SWATCH - 1, READOUT_SWATCH - 1, 2);
  if (swatch === 'hollow') {
    ctx.strokeStyle = theme.lineSoft;
    ctx.lineWidth = 1;
    ctx.stroke();
  } else {
    ctx.fillStyle = swatch;
    ctx.fill();
  }
  ctx.restore();
}

function drawRowReadout(
  ctx: Canvas2D,
  row: HeatLaneRow,
  x: number,
  y: number,
  width: number,
  rowHeight: number,
  theme: FamilyTheme,
): void {
  const readout = row.readout;
  if (!readout || width <= 0) return;
  const middleY = y + rowHeight / 2;
  if (readout.kind === 'missing') {
    drawMissingMark(ctx, x, middleY, width, theme, { figure: readout.figure, align: 'left' });
    return;
  }
  ctx.save();
  ctx.textBaseline = 'middle';
  ctx.textAlign = 'left';
  if (readout.kind === 'clear') {
    ctx.font = `${LABEL_WEIGHT} ${theme.fontSize.body}px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.fillText(ellipsise(readout.label, width, s => textWidth(ctx, s)), x, middleY);
    ctx.restore();
    return;
  }
  let cursor = x;
  if (readout.swatch) {
    drawReadoutSwatch(ctx, cursor, middleY, readout.swatch, theme);
    cursor += READOUT_SWATCH + READOUT_GAP;
  }
  const color = readout.color
    ?? (readout.tone === 'positive'
      ? theme.green
      : readout.tone === 'negative'
        ? theme.red
        : readout.tone === 'muted'
          ? theme.muted
          : theme.text);
  const secondaryColor = mix(theme.text, theme.surface, 0.25);
  if (readout.form === 'word') {
    ctx.font = `${VALUE_WEIGHT} ${theme.fontSize.body}px ${theme.font}`;
    ctx.fillStyle = color;
    const room = Math.max(0, x + width - cursor);
    const primary = ellipsise(readout.primary, room, s => textWidth(ctx, s));
    ctx.fillText(primary, cursor, middleY);
    if (readout.secondary) {
      const after = cursor + textWidth(ctx, primary) + 6;
      ctx.font = `${LABEL_WEIGHT} ${theme.fontSize.small}px ${theme.font}`;
      ctx.fillStyle = theme.muted;
      const rest = x + width - after;
      if (rest > 12) ctx.fillText(ellipsise(readout.secondary, rest, s => textWidth(ctx, s)), after, middleY);
    }
    ctx.restore();
    return;
  }
  ctx.font = `${readout.tone === 'muted' ? LABEL_WEIGHT : VALUE_WEIGHT} ${theme.fontSize.body}px ${theme.font}`;
  ctx.fillStyle = color;
  const slot = Math.min(READOUT_VALUE_SLOT, Math.max(0, x + width - cursor));
  if (readout.primary) {
    ctx.textAlign = 'right';
    ctx.fillText(ellipsise(readout.primary, slot, s => textWidth(ctx, s)), cursor + slot, middleY);
    ctx.textAlign = 'left';
  }
  if (readout.secondary) {
    const after = cursor + slot + READOUT_GAP;
    const rest = x + width - after;
    ctx.font = `400 ${theme.fontSize.body}px ${theme.font}`;
    ctx.fillStyle = secondaryColor;
    if (rest > 12) ctx.fillText(ellipsise(readout.secondary, rest, s => textWidth(ctx, s)), after, middleY);
  }
  ctx.restore();
}

function drawInRunLabel(
  ctx: Canvas2D,
  text: string,
  x: number,
  y: number,
  width: number,
  height: number,
  bgColor: string,
  theme: FamilyTheme,
  explicitTextColor?: string,
): void {
  if (!text || width < 28 || height < 14) return;
  ctx.save();
  ctx.font = `${LABEL_WEIGHT} ${theme.fontSize.small}px ${theme.font}`;
  const needed = textWidth(ctx, text) + 16;
  if (width < needed) {
    ctx.restore();
    return;
  }
  ctx.fillStyle = explicitTextColor ?? contrastInkForFill(bgColor, theme);
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  ctx.fillText(text, x + 8, y + height / 2 + 0.5);
  ctx.restore();
}

export function drawHeatLanes(ctx: Canvas2D, input: DrawHeatLanesInput): void {
  const { model, layout, theme, scrollTop, hoveredRow, hoveredHour, selectedRowId, focusedRowId, artProvider } = input;
  const tokens = resolveKitTokens(theme);

  ctx.save();
  ctx.fillStyle = theme.surface;
  ctx.fillRect(0, 0, layout.width, layout.height);

  if (model.legend && layout.legendHeight > 0) {
    drawLegendStrip(ctx, 0, 0, layout.width, layout.legendHeight, {
      gradient: model.legend.gradient,
      entries: model.legend.entries?.map(entry => ({
        label: entry.label,
        color: entry.color,
        ...(entry.hatched ? { hatch: true } : {}),
        ...(entry.dashed ? { dash: [4, 2] } : {}),
        ...(entry.muted ? { muted: true } : {}),
      })),
      rightNote: model.legend.rightNote,
      padX: LANE_PAD,
      hairline: true,
    }, theme);
  }

  const bodyBottom = layout.height - layout.axisHeight;
  ctx.save();
  ctx.beginPath();
  ctx.rect(0, layout.plotTop, layout.width, Math.max(0, bodyBottom - layout.plotTop));
  ctx.clip();

  const laneBarPad = 2;
  const laneBarHeight = Math.max(8, layout.rowHeight - laneBarPad * 2);
  const laneRadius = 3;

  const anyPinned = model.rows.some(row => row.pinned);
  const autoNameColor = mix(theme.text, theme.surface, 0.25);
  for (let i = layout.firstRow; i < layout.endRow; i++) {
    const row = model.rows[i]!;
    const y = layout.plotTop + i * layout.rowStride - scrollTop;
    const isHovered = hoveredRow === i;
    const isSelected = selectedRowId === row.id;
    const isFocused = focusedRowId === row.id;

    if (isSelected) {
      drawRowSelected(ctx, 0, y, layout.width, layout.rowHeight, theme, 2);
    } else if (isHovered) {
      drawRowHover(ctx, 0, y, layout.width, layout.rowHeight, theme, 2);
    }
    if (isFocused) {
      drawRowFocusRing(ctx, 0, y, layout.width, layout.rowHeight, theme, 2);
    }

    let nameX = LANE_PAD;
    if (layout.showFlags && (row.flag !== undefined || row.tag !== undefined)) {
      drawRowFlag(ctx, row, nameX, y, layout.rowHeight, theme, artProvider);
      nameX += FLAG_SLOT_WIDTH + 6;
    }
    const nameMaxW = Math.max(16, layout.nameWidth - nameX);
    drawNameSlot(ctx, row.label, nameX, y, nameMaxW, layout.rowHeight, theme, {
      fontSize: theme.fontSize.body,
      weight: 400,
      maxLines: 1,
      color: row.nameMissing ? theme.muted : anyPinned && !row.pinned ? autoNameColor : theme.text,
    });

    const barY = y + laneBarPad;
    ctx.save();
    ctx.fillStyle = tokens.barTrack;
    ctx.beginPath();
    ctx.roundRect(layout.plotLeft, barY, layout.plotWidth, laneBarHeight, laneRadius);
    ctx.fill();
    ctx.clip();

    const spans = projectLaneRunsToPixels(
      row.runs,
      layout.firstHour,
      layout.lastHour,
      layout.displayedHour,
      layout.plotLeft,
      layout.plotWidth,
    );

    for (let s = 0; s < spans.length; s++) {
      const span = spans[s]!;
      if (span.width <= 0) continue;
      const cell = span.cell;
      if (cell.kind === 'clear') continue;
      if (cell.kind === 'gap') {
        drawGapBand(ctx, span.x, barY, span.width, laneBarHeight, theme);
        continue;
      }
      if (cell.kind === 'missing') {
        drawHatchRect(ctx, span.x, barY, span.width, laneBarHeight, 0, theme);
        continue;
      }
      if (cell.kind === 'solid') {
        ctx.fillStyle = cell.color;
        ctx.fillRect(span.x, barY, span.width, laneBarHeight);
        if (cell.label) {
          drawInRunLabel(ctx, cell.label, span.x, barY, span.width, laneBarHeight, cell.color, theme, cell.textColor);
        }
        continue;
      }
      if (cell.look === 'split') {
        const stripes = splitStripes(cell.parts, barY, laneBarHeight);
        let filled = 0;
        for (const stripe of stripes) {
          ctx.fillStyle = stripe.color;
          ctx.fillRect(span.x, stripe.top, span.width, stripe.height);
          filled += stripe.height;
        }
        if (filled < laneBarHeight - 0.5) drawHatchRect(ctx, span.x, barY + filled, span.width, laneBarHeight - filled, 0, theme);
        if (cell.label && stripes.length === 1) {
          drawInRunLabel(ctx, cell.label, span.x, barY, span.width, laneBarHeight, stripes[0]!.color, theme);
        }
      } else {
        const fill = cell.color ?? marginColour(cell.parts, theme.grey);
        if (fill) {
          ctx.fillStyle = fill;
          ctx.fillRect(span.x, barY, span.width, laneBarHeight);
          if (cell.label) {
            drawInRunLabel(ctx, cell.label, span.x, barY, span.width, laneBarHeight, fill, theme);
          }
        }
      }
    }
    ctx.restore();

    if (layout.showReadout) {
      drawRowReadout(
        ctx,
        row,
        layout.plotLeft + layout.plotWidth + LANE_PAD,
        y,
        layout.readoutWidth - LANE_PAD,
        layout.rowHeight,
        theme,
      );
    }
  }

  const rowsBottom = Math.min(bodyBottom, layout.plotTop + model.rows.length * layout.rowStride - LANE_ROW_GAP - scrollTop);
  if (model.gaps?.length && rowsBottom > layout.plotTop) {
    for (const gap of model.gaps) {
      if (gap.firstHour > layout.displayedHour) continue;
      const gFirst = Math.max(layout.firstHour, gap.firstHour);
      const gLast = Math.min(layout.lastHour, layout.displayedHour, gap.lastHour);
      if (gFirst > gLast) continue;
      const gx0 = hourToLaneX(gFirst, layout.firstHour, layout.lastHour, layout.plotLeft, layout.plotWidth);
      const gx1 = hourToLaneX(gLast + 1, layout.firstHour, layout.lastHour, layout.plotLeft, layout.plotWidth);
      if (gx1 > gx0) {
        drawGapBand(ctx, gx0, layout.plotTop, Math.max(1, gx1 - gx0), rowsBottom - layout.plotTop, theme);
      }
    }
  }

  if (model.playhead !== null && model.playhead >= layout.firstHour && model.playhead <= layout.lastHour) {
    const px = hourToLaneX(model.playhead, layout.firstHour, layout.lastHour, layout.plotLeft, layout.plotWidth);
    drawPlayheadLine(ctx, px, layout.plotTop, bodyBottom, theme);
  }

  if (hoveredHour !== null && hoveredHour >= layout.firstHour && hoveredHour <= layout.displayedHour) {
    const hx = hourToLaneX(hoveredHour, layout.firstHour, layout.lastHour, layout.plotLeft, layout.plotWidth);
    drawCrosshairLine(ctx, hx, layout.plotTop, bodyBottom, theme);
  }

  ctx.restore();

  drawRowHairline(ctx, 0, layout.width, bodyBottom, theme, 'normal');
  const ticks = computeTimeAxisTicks(layout.firstHour, layout.lastHour, layout.plotWidth, model.dayHours);
  const playheadInRange = model.playhead !== null && model.playhead >= layout.firstHour && model.playhead <= layout.lastHour;
  const hoverInRange = hoveredHour !== null && hoveredHour >= layout.firstHour && hoveredHour <= layout.displayedHour;
  drawTimeAxis(ctx, ticks, {
    plotLeft: layout.plotLeft,
    plotRight: layout.plotLeft + layout.plotWidth,
    startHour: layout.firstHour,
    endHour: layout.lastHour,
    axisTop: bodyBottom,
    labelY: bodyBottom + layout.axisHeight / 2,
    tabY: bodyBottom + 1,
    playhead: playheadInRange ? model.playhead : null,
    playheadLabel: playheadInRange ? formatDateOnly(model.playhead!) : '',
    hover: hoverInRange ? hoveredHour : null,
    hoverLabel: hoverInRange ? formatDateOnly(hoveredHour!) : undefined,
    playheadLineTop: null,
    hairline: 'none',
    tickMarks: false,
    theme,
  });

  ctx.restore();
}
