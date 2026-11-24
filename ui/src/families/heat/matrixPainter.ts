import { LABEL_WEIGHT, VALUE_WEIGHT } from '../../charts/draw.ts';
import { ellipsise } from '../../charts/labels.ts';
import { textWidth, type Canvas2D } from '../../charts/textWidth.ts';
import {
  drawBar,
  drawFlagSlot,
  drawGradientLegend,
  drawLegendEntry,
  drawNameSlot,
  drawRowFocusRing,
  drawRowHairline,
  drawRowHover,
  FLAG_SLOT_HEIGHT,
  FLAG_SLOT_WIDTH,
  resolveKitTokens,
} from '../../draw/kit.ts';
import { mix, parseColor, readableText } from '../../shell/appearance/color.ts';
import { toFlagSlotState } from '../art/adapters.ts';
import type { LaneArtCache } from '../art/laneCache.ts';
import { artUrlForHandle } from '../art/provider.ts';
import type { ArtProvider, FamilyTheme } from '../contract.ts';
import type { IdentityIndex } from '../identity/index.ts';
import { drawHatchRect } from '../valueCanvas.ts';
import type { HeatMatrixLayout } from './matrixLayout.ts';
import { MATRIX_COLUMN_NAME_ANGLE, MATRIX_COLUMN_NAME_HEIGHT, MATRIX_PAD } from './matrixLayout.ts';
import type { HeatMatrixModel, MatrixAxisEntity, MatrixRamp } from './matrixModel.ts';

export interface HeatMatrixPicture {
  signature: string;
}

export function createMatrixPicture(): HeatMatrixPicture {
  return { signature: '' };
}

export interface DrawHeatMatrixInput {
  readonly model: HeatMatrixModel;
  readonly layout: HeatMatrixLayout;
  readonly theme: FamilyTheme;
  readonly scrollLeft: number;
  readonly scrollTop: number;
  readonly hoveredCell: { readonly row: number; readonly column: number } | null;
  readonly focusedCell: { readonly row: number; readonly column: number } | null;
  readonly artProvider?: ArtProvider;
  readonly artCache?: LaneArtCache;
  readonly identityIndex?: IdentityIndex | null;
}

export function collectVisibleMatrixArt(
  model: HeatMatrixModel,
  layout: HeatMatrixLayout,
  identityIndex?: IdentityIndex | null,
): readonly string[] {
  const urls = new Set<string>();
  const add = (entity: MatrixAxisEntity) => {
    if (!entity.flag) return;
    const url = artUrlForHandle(entity.flag, identityIndex);
    if (url) urls.add(url);
  };
  for (let r = layout.firstRow; r < layout.endRow; r++) {
    const entity = model.rows[r];
    if (entity) add(entity);
  }
  for (let c = layout.firstCol; c < layout.endCol; c++) {
    const entity = model.columns[c];
    if (entity) add(entity);
  }
  return [...urls];
}

function drawLegendStrip(
  ctx: Canvas2D,
  model: HeatMatrixModel,
  layout: HeatMatrixLayout,
  theme: FamilyTheme,
): void {
  const legend = model.legend;
  if (!legend || layout.legendHeight <= 0) return;
  const middleY = Math.round(layout.legendHeight / 2);
  let cursor = MATRIX_PAD;
  if (legend.gradient) {
    const stops = legend.gradient.stops.length ? legend.gradient.stops : [rampColour(theme, model.ramp, 0), rampColour(theme, model.ramp, 1)];
    const drawn = drawGradientLegend(
      ctx,
      cursor,
      middleY,
      legend.gradient.lowLabel,
      legend.gradient.highLabel,
      stops,
      theme,
    );
    cursor += drawn.width + 14;
  }
  if (legend.entries) {
    for (const entry of legend.entries) {
      if (cursor > layout.width - 40) break;
      if (entry.hatched || entry.outlined) {
        if (entry.hatched) {
          drawHatchRect(ctx, cursor, Math.round(middleY - 4.5), 9, 9, 2, theme);
        } else {
          ctx.save();
          ctx.strokeStyle = theme.muted;
          ctx.lineWidth = 1;
          ctx.setLineDash([2, 2]);
          ctx.beginPath();
          ctx.roundRect(cursor + 0.5, Math.round(middleY - 4.5) + 0.5, 8, 8, 2);
          ctx.stroke();
          ctx.restore();
        }
        ctx.save();
        ctx.font = `${LABEL_WEIGHT} ${theme.fontSize.small}px ${theme.font}`;
        ctx.fillStyle = theme.muted;
        ctx.textAlign = 'left';
        ctx.textBaseline = 'middle';
        ctx.fillText(entry.label, cursor + 15, middleY);
        cursor += 15 + textWidth(ctx, entry.label) + 12;
        ctx.restore();
      } else {
        const drawn = drawLegendEntry(ctx, cursor, middleY, entry.label, entry.color, theme, {
          maxWidth: Math.max(40, layout.width - cursor - MATRIX_PAD),
        });
        cursor += drawn.width + 12;
      }
    }
  }
  if (legend.rightNote) {
    ctx.save();
    ctx.font = `${LABEL_WEIGHT} ${theme.fontSize.small}px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.textAlign = 'right';
    ctx.textBaseline = 'middle';
    const maxW = Math.max(0, layout.width - cursor - MATRIX_PAD);
    if (maxW >= 24) {
      ctx.fillText(ellipsise(legend.rightNote, maxW, s => textWidth(ctx, s)), layout.width - MATRIX_PAD, middleY);
    }
    ctx.restore();
  }
  drawRowHairline(ctx, 0, layout.width, layout.legendHeight - 1, theme, 'normal');
}

function resolveEntityFlag(
  entity: MatrixAxisEntity,
  artProvider?: ArtProvider,
) {
  if (entity.flag) {
    if (artProvider) {
      return toFlagSlotState(artProvider.resolve(entity.flag), entity.tag ?? entity.id);
    }
    return { kind: 'pending' as const };
  }
  if (entity.flagPending) return { kind: 'pending' as const };
  return { kind: 'missing' as const, tag: entity.tag ?? entity.id };
}

export function rampColour(theme: FamilyTheme, ramp: MatrixRamp | undefined, intensity: number): string {
  const tone = ramp === 'red' ? theme.red : ramp === 'amber' ? theme.amber : theme.accent;
  return mix(theme.surface, tone, 0.22 + 0.78 * Math.max(0, Math.min(1, intensity)));
}

function drawCellText(ctx: Canvas2D, text: string, x: number, y: number, w: number, h: number, color: string, theme: FamilyTheme): void {
  if (w < 18 || h < 11) return;
  ctx.fillStyle = color;
  ctx.font = `${VALUE_WEIGHT} ${Math.max(9, Math.min(11, Math.floor(Math.min(w * 0.36, h * 0.72))))}px ${theme.font}`;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  ctx.fillText(ellipsise(text, w - 4, s => textWidth(ctx, s)), x + w / 2, y + h / 2 + 0.5);
}

function drawMarginColumn(
  ctx: Canvas2D,
  model: HeatMatrixModel,
  layout: HeatMatrixLayout,
  theme: FamilyTheme,
  scrollTop: number,
  hoveredRow: number | null,
): void {
  const margin = model.margin;
  if (!margin || layout.marginWidth <= 0) return;
  const left = layout.width - layout.marginWidth;
  const ch = layout.cellHeight;
  ctx.save();
  ctx.fillStyle = theme.surface;
  ctx.fillRect(left, layout.legendHeight, layout.marginWidth, layout.height - layout.legendHeight);
  ctx.font = `${LABEL_WEIGHT} ${theme.fontSize.small}px ${theme.font}`;
  ctx.fillStyle = theme.muted;
  ctx.textAlign = 'right';
  ctx.textBaseline = 'bottom';
  ctx.fillText(ellipsise(margin.header, layout.marginWidth - MATRIX_PAD * 2, s => textWidth(ctx, s)), layout.width - MATRIX_PAD, layout.gridTop - 4);
  ctx.beginPath();
  ctx.rect(left, layout.gridTop, layout.marginWidth, Math.max(0, layout.height - layout.gridTop));
  ctx.clip();
  const barLeft = left + MATRIX_PAD + 4;
  const figureWidth = 44;
  const barMax = Math.max(8, layout.marginWidth - MATRIX_PAD * 2 - 4 - figureWidth - 6);
  const barHeight = Math.max(3, Math.min(5, Math.round(ch * 0.28)));
  for (let r = layout.firstRow; r < layout.endRow; r++) {
    const y = layout.gridTop + r * ch - scrollTop;
    if (hoveredRow === r) drawRowHover(ctx, left, y, layout.marginWidth, ch, theme, 0);
    const item = margin.cell(r);
    const middle = y + ch / 2;
    if (item.kind === 'missing') {
      drawHatchRect(ctx, barLeft, Math.round(middle - 3), barMax, 6, 2, theme);
      continue;
    }
    if (item.kind === 'none') {
      ctx.fillStyle = theme.muted;
      ctx.textAlign = 'right';
      ctx.textBaseline = 'middle';
      ctx.fillText(item.label, layout.width - MATRIX_PAD, middle + 0.5);
      continue;
    }
    drawBar(ctx, barLeft, Math.round(middle - barHeight / 2), Math.max(2, barMax * Math.max(0, Math.min(1, item.fraction))), barHeight, item.color ?? rampColour(theme, model.ramp, 1), 'right');
    ctx.font = `${VALUE_WEIGHT} ${theme.fontSize.small}px ${theme.font}`;
    ctx.fillStyle = theme.text;
    ctx.textAlign = 'right';
    ctx.textBaseline = 'middle';
    ctx.fillText(ellipsise(item.text, figureWidth, s => textWidth(ctx, s)), layout.width - MATRIX_PAD, middle + 0.5);
  }
  ctx.restore();
}

function drawColumnName(ctx: Canvas2D, label: string, x: number, bottom: number, theme: FamilyTheme, muted: boolean): void {
  const room = MATRIX_COLUMN_NAME_HEIGHT / Math.sin(MATRIX_COLUMN_NAME_ANGLE) - 6;
  ctx.save();
  ctx.translate(x, bottom);
  ctx.rotate(-MATRIX_COLUMN_NAME_ANGLE);
  ctx.font = `${LABEL_WEIGHT} ${theme.fontSize.small}px ${theme.font}`;
  ctx.fillStyle = muted ? theme.muted : theme.text;
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  ctx.fillText(ellipsise(label, room, s => textWidth(ctx, s)), 0, 0);
  ctx.restore();
}

export function drawHeatMatrix(
  ctx: Canvas2D,
  input: DrawHeatMatrixInput,
  picture?: HeatMatrixPicture,
): { readonly drew: boolean } {
  const { model, layout, theme, scrollLeft, scrollTop, hoveredCell, focusedCell, artProvider, artCache } = input;
  const tokens = resolveKitTokens(theme);

  const artRev = artCache ? `${artCache.revision}:${artCache.cachedCount}:${artCache.failedCount}` : '';
  const sig = JSON.stringify([
    model.hour,
    layout.width,
    layout.height,
    layout.cellWidth,
    layout.cellHeight,
    layout.marginWidth,
    layout.firstRow,
    layout.endRow,
    layout.firstCol,
    layout.endCol,
    Math.round(scrollLeft),
    Math.round(scrollTop),
    hoveredCell?.row ?? -1,
    hoveredCell?.column ?? -1,
    focusedCell?.row ?? -1,
    focusedCell?.column ?? -1,
    theme.surface,
    theme.text,
    theme.accent,
    artRev,
  ]);
  if (picture && picture.signature === sig) {
    return { drew: false };
  }
  if (picture) picture.signature = sig;

  ctx.save();
  ctx.fillStyle = theme.surface;
  ctx.fillRect(0, 0, layout.width, layout.height);

  drawLegendStrip(ctx, model, layout, theme);

  const cw = layout.cellWidth;
  const ch = layout.cellHeight;
  const innerW = Math.max(4, cw - 2);
  const innerH = Math.max(4, ch - 2);
  const radius = layout.wide ? 2 : 3;
  const gridRight = layout.width - layout.marginWidth;
  const hoveredColumn = hoveredCell && hoveredCell.column < model.columns.length ? hoveredCell.column : null;

  ctx.save();
  ctx.beginPath();
  ctx.rect(layout.gridLeft, layout.gridTop, Math.max(0, gridRight - layout.gridLeft), Math.max(0, layout.height - layout.gridTop));
  ctx.clip();

  if (hoveredCell) {
    const hy = layout.gridTop + hoveredCell.row * ch - scrollTop;
    ctx.fillStyle = tokens.rowHover;
    ctx.fillRect(layout.gridLeft, hy, gridRight - layout.gridLeft, ch);
    if (hoveredColumn !== null) {
      const hx = layout.gridLeft + hoveredColumn * cw - scrollLeft;
      ctx.fillRect(hx, layout.gridTop, cw, layout.height - layout.gridTop);
    }
  }

  for (let r = layout.firstRow; r < layout.endRow; r++) {
    const y = layout.gridTop + r * ch - scrollTop;
    for (let c = layout.firstCol; c < layout.endCol; c++) {
      const x = layout.gridLeft + c * cw - scrollLeft;
      const item = model.cell(r, c);
      const cx = x + 1;
      const cy = y + 1;
      if (item.kind === 'diagonal') {
        if (item.outlined) {
          ctx.strokeStyle = theme.muted;
          ctx.globalAlpha = 0.7;
          ctx.lineWidth = 1;
          ctx.setLineDash([3, 2]);
          ctx.beginPath();
          ctx.roundRect(cx + 0.5, cy + 0.5, innerW - 1, innerH - 1, radius);
          ctx.stroke();
          ctx.setLineDash([]);
          ctx.globalAlpha = 1;
          if (item.text) drawCellText(ctx, item.text, cx, cy, innerW, innerH, theme.muted, theme);
        } else if (item.text) {
          drawCellText(ctx, item.text, cx, cy, innerW, innerH, theme.muted, theme);
        } else {
          ctx.fillStyle = tokens.headerBand;
          ctx.beginPath();
          ctx.roundRect(cx, cy, innerW, innerH, radius);
          ctx.fill();
          ctx.strokeStyle = tokens.hairline;
          ctx.lineWidth = 1;
          ctx.beginPath();
          ctx.moveTo(cx + 4, cy + innerH - 4);
          ctx.lineTo(cx + innerW - 4, cy + 4);
          ctx.stroke();
        }
      } else if (item.kind === 'missing') {
        drawHatchRect(ctx, cx, cy, innerW, innerH, radius, theme);
      } else if (item.kind === 'empty') {
        ctx.fillStyle = tokens.barTrack;
        ctx.beginPath();
        ctx.roundRect(cx, cy, innerW, innerH, radius);
        ctx.fill();
      } else {
        const fill = item.color ?? rampColour(theme, model.ramp, item.intensity ?? 1);
        ctx.fillStyle = fill;
        ctx.beginPath();
        ctx.roundRect(cx, cy, innerW, innerH, radius);
        ctx.fill();
        if (item.text && innerW >= 24) {
          const rgb = parseColor(fill);
          drawCellText(ctx, item.text, cx, cy, innerW, innerH, rgb ? readableText(rgb) : theme.text, theme);
        }
      }

      if (hoveredCell && hoveredCell.row === r && hoveredCell.column === c) {
        ctx.strokeStyle = theme.text;
        ctx.lineWidth = 1.5;
        ctx.beginPath();
        ctx.roundRect(cx + 0.75, cy + 0.75, innerW - 1.5, innerH - 1.5, radius);
        ctx.stroke();
      }
      if (focusedCell && focusedCell.row === r && focusedCell.column === c) {
        drawRowFocusRing(ctx, x, y, cw, ch, theme, 0);
      }
    }
  }

  for (const span of layout.rowGroups) {
    if (span.first > 0 && span.first >= layout.firstRow && span.first <= layout.endRow) {
      const gy = layout.gridTop + span.first * ch - scrollTop;
      drawRowHairline(ctx, layout.gridLeft, gridRight, gy, theme, 'band');
    }
  }
  for (const span of layout.colGroups) {
    if (span.first > 0 && span.first >= layout.firstCol && span.first <= layout.endCol) {
      const gx = Math.floor(layout.gridLeft + span.first * cw - scrollLeft) + 0.5;
      ctx.save();
      ctx.strokeStyle = tokens.bandDivider;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(gx, layout.gridTop);
      ctx.lineTo(gx, layout.height);
      ctx.stroke();
      ctx.restore();
    }
  }
  ctx.restore();

  ctx.save();
  ctx.beginPath();
  ctx.rect(0, layout.gridTop, layout.gridLeft, Math.max(0, layout.height - layout.gridTop));
  ctx.clip();
  ctx.fillStyle = theme.surface;
  ctx.fillRect(0, layout.gridTop, layout.gridLeft, Math.max(0, layout.height - layout.gridTop));

  for (let r = layout.firstRow; r < layout.endRow; r++) {
    const entity = model.rows[r]!;
    const y = layout.gridTop + r * ch - scrollTop;
    if (hoveredCell?.row === r) {
      drawRowHover(ctx, 0, y, layout.gridLeft, ch, theme, 2);
    }
    let cursorX = layout.rowGroupWidth + MATRIX_PAD;
    if (layout.showFlags) {
      const flagHeight = Math.min(FLAG_SLOT_HEIGHT, ch - 4);
      const flagWidth = Math.round(flagHeight * (FLAG_SLOT_WIDTH / FLAG_SLOT_HEIGHT));
      const flagState = resolveEntityFlag(entity, artProvider);
      drawFlagSlot(ctx, cursorX, Math.round(y + (ch - flagHeight) / 2), flagState, theme, { width: flagWidth, height: flagHeight });
      cursorX += flagWidth + 6;
    }
    const nameRight = layout.gridLeft - MATRIX_PAD - (entity.color ? 4 : 0);
    if (layout.showNames) {
      const nameW = Math.max(16, nameRight - cursorX);
      drawNameSlot(ctx, entity.label, cursorX, y, nameW, ch, theme, {
        fontSize: theme.fontSize.small,
        maxLines: 1,
        color: entity.nameMissing ? theme.muted : theme.text,
      });
    } else if (!layout.showFlags) {
      const code = entity.shortLabel ?? entity.tag ?? entity.label;
      drawNameSlot(ctx, code, cursorX, y, Math.max(16, nameRight - cursorX + MATRIX_PAD - 4), ch, theme, {
        fontSize: theme.fontSize.small,
        maxLines: 1,
      });
    }
    if (entity.color) {
      ctx.fillStyle = entity.color;
      ctx.fillRect(layout.gridLeft - 5, y + 2, 2, Math.max(2, ch - 4));
    }
  }

  for (const span of layout.rowGroups) {
    const y0 = layout.gridTop + span.first * ch - scrollTop;
    const h = span.count * ch;
    if (y0 + h < layout.gridTop || y0 > layout.height) continue;
    ctx.fillStyle = span.color ?? tokens.bandDivider;
    ctx.fillRect(1, y0 + 2, 3, Math.max(4, h - 4));
    if (span.first > 0) {
      drawRowHairline(ctx, 0, layout.gridLeft, y0, theme, 'band');
    }
  }
  ctx.restore();

  ctx.save();
  ctx.beginPath();
  ctx.rect(layout.gridLeft, layout.legendHeight, Math.max(0, gridRight - layout.gridLeft), layout.headerHeight);
  ctx.clip();
  ctx.fillStyle = theme.surface;
  ctx.fillRect(layout.gridLeft, layout.legendHeight, Math.max(0, gridRight - layout.gridLeft), layout.headerHeight);

  if (layout.colGroupHeight > 0) {
    const bandY = layout.legendHeight;
    ctx.fillStyle = tokens.headerBand;
    ctx.fillRect(layout.gridLeft, bandY, Math.max(0, gridRight - layout.gridLeft), layout.colGroupHeight);
    for (const span of layout.colGroups) {
      const x0 = layout.gridLeft + span.first * cw - scrollLeft;
      const w = span.count * cw;
      if (x0 + w < layout.gridLeft || x0 > gridRight) continue;
      if (span.color) {
        ctx.fillStyle = span.color;
        ctx.fillRect(x0 + 2, bandY + layout.colGroupHeight - 2, Math.max(4, w - 4), 2);
      }
      ctx.font = `${LABEL_WEIGHT} 10px ${theme.font}`;
      ctx.fillStyle = theme.muted;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      const labelX = Math.max(layout.gridLeft + 4, x0 + 4);
      const avail = Math.max(0, x0 + w - labelX - 4);
      if (avail >= 16) {
        ctx.fillText(ellipsise(span.label, avail, s => textWidth(ctx, s)), labelX, bandY + layout.colGroupHeight / 2);
      }
      if (span.first > 0) {
        const px = Math.floor(x0) + 0.5;
        ctx.strokeStyle = tokens.bandDivider;
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.moveTo(px, bandY);
        ctx.lineTo(px, layout.gridTop);
        ctx.stroke();
      }
    }
    drawRowHairline(ctx, layout.gridLeft, gridRight, bandY + layout.colGroupHeight - 1, theme, 'normal');
  }

  const colTop = layout.legendHeight + layout.colGroupHeight;
  const namesHeight = layout.columnNames ? MATRIX_COLUMN_NAME_HEIGHT : 0;
  const flagBandTop = colTop + namesHeight;
  const flagBandH = layout.gridTop - flagBandTop;
  for (let c = layout.firstCol; c < layout.endCol; c++) {
    const entity = model.columns[c]!;
    const x = layout.gridLeft + c * cw - scrollLeft;
    if (hoveredColumn === c) {
      drawRowHover(ctx, x, colTop, cw, layout.gridTop - colTop, theme, 1);
    }
    if (layout.columnNames) {
      drawColumnName(ctx, entity.label, x + cw / 2 - 2, flagBandTop - 2, theme, Boolean(entity.nameMissing));
    }
    if (layout.showFlags) {
      const flagHeight = layout.wide ? 10 : FLAG_SLOT_HEIGHT;
      const flagWidth = layout.wide ? 16 : FLAG_SLOT_WIDTH;
      const fx = Math.round(x + (cw - flagWidth) / 2);
      const fy = Math.round(flagBandTop + (flagBandH - flagHeight) / 2) - (entity.color ? 1 : 0);
      drawFlagSlot(ctx, fx, fy, resolveEntityFlag(entity, artProvider), theme, { width: flagWidth, height: flagHeight });
    } else {
      const code = entity.shortLabel ?? entity.tag ?? entity.label;
      drawNameSlot(ctx, code, x + 2, flagBandTop, Math.max(8, cw - 4), flagBandH, theme, {
        fontSize: 10,
        align: 'center',
        maxLines: 1,
      });
    }
    if (entity.color) {
      ctx.fillStyle = entity.color;
      ctx.fillRect(x + 1, layout.gridTop - 3, Math.max(2, cw - 2), 2);
    }
  }
  ctx.restore();

  ctx.fillStyle = theme.surface;
  ctx.fillRect(0, layout.legendHeight, layout.gridLeft, layout.headerHeight);
  if (model.corner && layout.showNames) {
    ctx.save();
    ctx.font = `${LABEL_WEIGHT} ${theme.fontSize.small}px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.textAlign = 'left';
    ctx.textBaseline = 'bottom';
    ctx.fillText(ellipsise(model.corner, layout.gridLeft - MATRIX_PAD * 2, s => textWidth(ctx, s)), MATRIX_PAD, layout.gridTop - 4);
    ctx.restore();
  }
  drawMarginColumn(ctx, model, layout, theme, scrollTop, hoveredCell?.row ?? null);
  if (!layout.wide) {
    drawRowHairline(ctx, 0, layout.width, layout.gridTop - 1, theme, 'strong');
    const gx = Math.floor(layout.gridLeft) - 0.5;
    ctx.strokeStyle = tokens.hairlineStrong;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(gx, layout.legendHeight);
    ctx.lineTo(gx, layout.height);
    ctx.stroke();
  }

  ctx.restore();
  return { drew: true };
}
