import { LABEL_WEIGHT, VALUE_WEIGHT } from '../../charts/draw.ts';
import { ellipsiseCanvasText, measureCanvasText, type Canvas2D } from '../../charts/textWidth.ts';
import {
  computeTimeAxisTicks,
  contrastInkForFill,
  drawAddRow,
  drawCrosshairLine,
  drawFlagSlot,
  drawGapBand,
  drawLegendStrip,
  drawMissingMark,
  drawNameSlot,
  drawPlayheadLine,
  drawRowFocusRing,
  drawRowGrip,
  drawRowHairline,
  drawRowHover,
  drawRowRemoveButton,
  drawRowSectionLabel,
  drawRowSelected,
  drawTimeAxis,
  FLAG_SLOT_HEIGHT,
  FLAG_SLOT_WIDTH,
  resolveKitTokens,
  ROW_REMOVE_SIZE,
  ROW_SECTION_HEIGHT,
} from '../../draw/kit.ts';
import { REGION_ADD_KEY } from '../regions/regionRows.ts';
import { mix } from '../../shell/appearance/color.ts';
import { resolveCachedArt, toArtSlotState } from '../art/adapters.ts';
import type { LaneArtCache } from '../art/laneCache.ts';
import { collectArtUrls } from '../art/provider.ts';
import type { FamilyTheme } from '../contract.ts';
import { drawHatchRect } from '../valueCanvas.ts';
import { formatDateOnly } from '../valueFormat.ts';
import {
  hourToLaneX,
  LANE_PAD,
  projectLaneRunsToPixels,
  LanePixels,
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
  readonly hoveredAdd?: boolean;
  readonly hoveredHour: number | null;
  readonly selectedRowId: string | null;
  readonly focusedRowId: string | null;
  readonly artCache?: LaneArtCache;
}

export function collectVisibleLaneArt(
  model: HeatLanesModel,
  layout: HeatLanesLayout,
): readonly string[] {
  const flags = [];
  for (let i = layout.firstRow; i < layout.endRow; i++) {
    const row = model.rows[i];
    if (row?.flag) flags.push(row.flag);
  }
  return collectArtUrls(flags);
}

function drawRowFlag(
  ctx: Canvas2D,
  row: HeatLaneRow,
  x: number,
  y: number,
  rowHeight: number,
  theme: FamilyTheme,
  artCache?: LaneArtCache,
): void {
  if (!row.flag) return;
  const fy = Math.round(y + (rowHeight - FLAG_SLOT_HEIGHT) / 2);
  drawFlagSlot(ctx, x, fy, toArtSlotState(resolveCachedArt(row.flag, artCache), row.tag ?? row.id), theme);
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
    const clearFont = `${LABEL_WEIGHT} ${theme.fontSize.body}px ${theme.font}`;
    ctx.font = clearFont;
    ctx.fillStyle = theme.muted;
    ctx.fillText(ellipsiseCanvasText(ctx, readout.label, width, clearFont), x, middleY);
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
    const wordFont = `${VALUE_WEIGHT} ${theme.fontSize.body}px ${theme.font}`;
    ctx.font = wordFont;
    ctx.fillStyle = color;
    const room = Math.max(0, x + width - cursor);
    const primary = ellipsiseCanvasText(ctx, readout.primary, room, wordFont);
    ctx.fillText(primary, cursor, middleY);
    if (readout.secondary) {
      const after = cursor + measureCanvasText(ctx, primary, wordFont) + 6;
      const secFont = `${LABEL_WEIGHT} ${theme.fontSize.small}px ${theme.font}`;
      ctx.font = secFont;
      ctx.fillStyle = theme.muted;
      const rest = x + width - after;
      if (rest > 12) ctx.fillText(ellipsiseCanvasText(ctx, readout.secondary, rest, secFont), after, middleY);
    }
    ctx.restore();
    return;
  }
  const primaryFont = `${readout.tone === 'muted' ? LABEL_WEIGHT : VALUE_WEIGHT} ${theme.fontSize.body}px ${theme.font}`;
  ctx.font = primaryFont;
  ctx.fillStyle = color;
  const slot = Math.min(READOUT_VALUE_SLOT, Math.max(0, x + width - cursor));
  if (readout.primary) {
    ctx.textAlign = 'right';
    ctx.fillText(ellipsiseCanvasText(ctx, readout.primary, slot, primaryFont), cursor + slot, middleY);
    ctx.textAlign = 'left';
  }
  if (readout.secondary) {
    const after = cursor + slot + READOUT_GAP;
    const rest = x + width - after;
    const secFont = `400 ${theme.fontSize.body}px ${theme.font}`;
    ctx.font = secFont;
    ctx.fillStyle = secondaryColor;
    if (rest > 12) ctx.fillText(ellipsiseCanvasText(ctx, readout.secondary, rest, secFont), after, middleY);
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
  const runFont = `${LABEL_WEIGHT} ${theme.fontSize.small}px ${theme.font}`;
  const needed = measureCanvasText(ctx, text, runFont) + 16;
  if (width < needed) return;
  ctx.save();
  ctx.font = runFont;
  ctx.fillStyle = explicitTextColor ?? contrastInkForFill(bgColor, theme);
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  ctx.fillText(text, x + 8, y + height / 2 + 0.5);
  ctx.restore();
}

const paintedLanes = new WeakMap<Canvas2D, LanePixels[]>();

export function drawHeatLanes(ctx: Canvas2D, input: DrawHeatLanesInput): void {
  const { model, layout, theme, scrollTop, hoveredRow, hoveredAdd = false, hoveredHour, selectedRowId, focusedRowId, artCache } = input;
  const tokens = resolveKitTokens(theme);

  ctx.save();
  ctx.fillStyle = theme.surface;
  ctx.fillRect(0, 0, layout.width, layout.height);

  if (model.legend && layout.legendHeight > 0) {
    const rawEntries = model.legend.entries;
    let ordinaryLen = rawEntries?.length ?? 0;
    while (ordinaryLen > 0 && rawEntries![ordinaryLen - 1]!.hatched) ordinaryLen--;
    const trailing = rawEntries && ordinaryLen < rawEntries.length
      ? rawEntries.slice(ordinaryLen).map(entry => ({
          label: entry.label,
          color: entry.color,
          hatch: true,
          ...(entry.muted ? { muted: true } : {}),
        }))
      : undefined;
    drawLegendStrip(ctx, 0, 0, layout.width, layout.legendHeight, {
      gradient: model.legend.gradient,
      ...(rawEntries ? {
        entries: {
          length: ordinaryLen,
          at: i => {
            const entry = rawEntries[i]!;
            return {
              label: entry.label,
              color: entry.color,
              ...(entry.hatched ? { hatch: true } : {}),
              ...(entry.dashed ? { dash: [4, 2] } : {}),
              ...(entry.muted ? { muted: true } : {}),
            };
          },
          ...(trailing ? { trailing } : {}),
        },
      } : {}),
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

  const mutedAuto = model.regions !== undefined || model.rows.some(row => row.pinned);
  const autoNameColor = tokens.autoName;
  for (const section of layout.sections) {
    const sy = layout.plotTop + section.top - scrollTop;
    if (section.top < Math.round(scrollTop) || section.top + ROW_SECTION_HEIGHT > Math.round(scrollTop) + layout.plotHeight) continue;
    drawRowSectionLabel(ctx, LANE_PAD, sy, ROW_SECTION_HEIGHT, section.label, theme);
  }
  if (model.regions && layout.addVisible && layout.addTop !== null) {
    const ay = layout.plotTop + layout.addTop - scrollTop;
    if (hoveredAdd) drawRowHover(ctx, 0, ay, layout.width, layout.rowHeight, theme, 2);
    if (focusedRowId === REGION_ADD_KEY) drawRowFocusRing(ctx, 0, ay, layout.width, layout.rowHeight, theme, 2);
    drawAddRow(ctx, LANE_PAD, ay + layout.rowHeight / 2, layout.plotLeft, layout.plotLeft + layout.plotWidth, model.regions.addLabel, theme);
  }
  let pixels = paintedLanes.get(ctx);
  if (!pixels) paintedLanes.set(ctx, pixels = []);
  pixels.length = layout.endRow - layout.firstRow;
  for (let i = layout.firstRow; i < layout.endRow; i++) {
    const row = model.rows[i]!;
    const y = layout.plotTop + layout.rowTops[i]! - scrollTop;
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
      drawRowFlag(ctx, row, nameX, y, layout.rowHeight, theme, artCache);
      nameX += FLAG_SLOT_WIDTH + 6;
    }
    const tools = row.region !== undefined && (isHovered || isFocused);
    const nameMaxW = Math.max(16, layout.nameWidth - nameX - (tools ? ROW_REMOVE_SIZE + 4 : 0));
    drawNameSlot(ctx, row.label, nameX, y, nameMaxW, layout.rowHeight, theme, {
      fontSize: theme.fontSize.body,
      weight: 400,
      maxLines: 1,
      color: row.nameMissing ? theme.muted : mutedAuto && !row.pinned ? autoNameColor : theme.text,
    });
    if (tools) {
      drawRowGrip(ctx, LANE_PAD, y + layout.rowHeight / 2, theme);
      drawRowRemoveButton(ctx, layout.nameWidth, y + layout.rowHeight / 2, theme);
    }

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
      pixels[i - layout.firstRow] ??= new LanePixels(),
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

  if (model.gaps?.length && layout.firstRow < layout.endRow) {
    const rowsTop = layout.plotTop + layout.rowTops[layout.firstRow]! - scrollTop;
    const rowsBottom = layout.plotTop + layout.rowTops[layout.endRow - 1]! + layout.rowHeight - scrollTop;
    for (const gap of model.gaps) {
      if (gap.firstHour > layout.displayedHour) continue;
      const gFirst = Math.max(layout.firstHour, gap.firstHour);
      const gLast = Math.min(layout.lastHour, layout.displayedHour, gap.lastHour);
      if (gFirst > gLast) continue;
      const gx0 = hourToLaneX(gFirst, layout.firstHour, layout.lastHour, layout.plotLeft, layout.plotWidth);
      const gx1 = hourToLaneX(gLast + 1, layout.firstHour, layout.lastHour, layout.plotLeft, layout.plotWidth);
      if (gx1 > gx0) {
        drawGapBand(ctx, gx0, rowsTop, Math.max(1, gx1 - gx0), rowsBottom - rowsTop, theme);
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
    toX: hour => hourToLaneX(hour, layout.firstHour, layout.lastHour, layout.plotLeft, layout.plotWidth),
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
