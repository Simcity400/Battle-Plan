import { LABEL_WEIGHT, VALUE_WEIGHT } from '../../charts/draw.ts';
import { ellipsise } from '../../charts/labels.ts';
import { textWidth, type Canvas2D } from '../../charts/textWidth.ts';
import {
  drawFlagSlot,
  drawGradientLegend,
  drawLegendEntry,
  drawNameSlot,
  drawProgressRing,
  drawRowFocusRing,
  drawRowHairline,
  drawRowHover,
  FLAG_SLOT_HEIGHT,
  FLAG_SLOT_WIDTH,
  resolveKitTokens,
} from '../../draw/kit.ts';
import { parseColor, readableText } from '../../shell/appearance/color.ts';
import { toFlagSlotState } from '../art/adapters.ts';
import type { LaneArtCache } from '../art/laneCache.ts';
import { artUrlForHandle } from '../art/provider.ts';
import type { ArtProvider, FamilyTheme } from '../contract.ts';
import type { IdentityIndex } from '../identity/index.ts';
import { drawHatchRect } from '../valueCanvas.ts';
import type { HeatMatrixLayout } from './matrixLayout.ts';
import { MATRIX_PAD, MATRIX_TRAILING_GAP } from './matrixLayout.ts';

const MARK_SQUARE_PX = 14;
const MARK_RING_RADIUS_PX = 6.2;
const MARK_DOT_RADIUS_PX = 1.2;
import type { HeatMatrixModel, MatrixAxisEntity } from './matrixModel.ts';

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
    const drawn = drawGradientLegend(
      ctx,
      cursor,
      middleY,
      legend.gradient.lowLabel,
      legend.gradient.highLabel,
      legend.gradient.stops,
      theme,
    );
    cursor += drawn.width + 14;
  }
  if (legend.entries) {
    for (const entry of legend.entries) {
      if (cursor > layout.width - 40) break;
      if (entry.hatched) {
        drawHatchRect(ctx, cursor, Math.round(middleY - 4.5), 9, 9, 2, theme);
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
    layout.cellSize,
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

  const cell = layout.cellSize;
  const gap = 2;
  const inner = Math.max(4, cell - gap);
  const radius = 3;

  ctx.save();
  ctx.beginPath();
  ctx.rect(layout.gridLeft, layout.gridTop, Math.max(0, layout.width - layout.gridLeft), Math.max(0, layout.height - layout.gridTop));
  ctx.clip();

  if (hoveredCell) {
    const hy = layout.gridTop + hoveredCell.row * cell - scrollTop;
    const hx = layout.gridLeft + hoveredCell.column * cell - scrollLeft;
    ctx.fillStyle = tokens.rowHover;
    ctx.fillRect(layout.gridLeft, hy, layout.width - layout.gridLeft, cell);
    ctx.fillRect(hx, layout.gridTop, cell, layout.height - layout.gridTop);
  }

  for (let r = layout.firstRow; r < layout.endRow; r++) {
    const y = layout.gridTop + r * cell - scrollTop;
    for (let c = layout.firstCol; c < layout.endCol; c++) {
      const x = layout.gridLeft + c * cell - scrollLeft;
      const item = model.cell(r, c);
      const cx = x + 1;
      const cy = y + 1;
      if (item.kind === 'diagonal') {
        ctx.fillStyle = tokens.headerBand;
        ctx.beginPath();
        ctx.roundRect(cx, cy, inner, inner, radius);
        ctx.fill();
        ctx.strokeStyle = tokens.hairline;
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.moveTo(cx + 4, cy + inner - 4);
        ctx.lineTo(cx + inner - 4, cy + 4);
        ctx.stroke();
      } else if (item.kind === 'missing') {
        drawHatchRect(ctx, cx, cy, inner, inner, radius, theme);
      } else if (item.kind === 'empty') {
        if (item.mark === 'dot') {
          ctx.fillStyle = theme.muted;
          ctx.beginPath();
          ctx.arc(x + cell / 2, y + cell / 2, MARK_DOT_RADIUS_PX, 0, Math.PI * 2);
          ctx.fill();
        } else {
          ctx.fillStyle = tokens.barTrack;
          ctx.beginPath();
          ctx.roundRect(cx, cy, inner, inner, radius);
          ctx.fill();
        }
      } else if (item.mark === 'square') {
        const side = Math.min(MARK_SQUARE_PX, inner);
        ctx.fillStyle = item.color;
        ctx.beginPath();
        ctx.roundRect(x + (cell - side) / 2, y + (cell - side) / 2, side, side, radius);
        ctx.fill();
      } else if (item.mark === 'ring') {
        drawProgressRing(ctx, x + cell / 2, y + cell / 2, MARK_RING_RADIUS_PX, item.fraction ?? 0, item.color, theme);
      } else {
        ctx.fillStyle = item.color;
        ctx.beginPath();
        ctx.roundRect(cx, cy, inner, inner, radius);
        ctx.fill();
        if (item.text && inner >= 24) {
          const rgb = parseColor(item.color);
          ctx.fillStyle = rgb ? readableText(rgb) : theme.text;
          ctx.font = `${VALUE_WEIGHT} ${Math.max(9, Math.min(11, Math.floor(inner * 0.36)))}px ${theme.font}`;
          ctx.textAlign = 'center';
          ctx.textBaseline = 'middle';
          const shown = ellipsise(item.text, inner - 4, s => textWidth(ctx, s));
          ctx.fillText(shown, cx + inner / 2, cy + inner / 2 + 0.5);
        }
      }

      if (hoveredCell && hoveredCell.row === r && hoveredCell.column === c) {
        ctx.strokeStyle = theme.text;
        ctx.lineWidth = 1.5;
        ctx.beginPath();
        ctx.roundRect(cx + 0.75, cy + 0.75, inner - 1.5, inner - 1.5, radius);
        ctx.stroke();
      }
      if (focusedCell && focusedCell.row === r && focusedCell.column === c) {
        drawRowFocusRing(ctx, x, y, cell, cell, theme, 0);
      }
    }
    const trailing = model.rows[r]!.trailing;
    if (trailing && layout.trailingWidth > 0) {
      const tx = layout.gridLeft + model.columns.length * cell - scrollLeft + MATRIX_TRAILING_GAP;
      ctx.font = `${LABEL_WEIGHT} ${theme.fontSize.small}px ${theme.font}`;
      ctx.fillStyle = theme.muted;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillText(trailing, tx, y + cell / 2 + 0.5);
    }
  }

  for (const span of layout.rowGroups) {
    if (span.first > 0 && span.first >= layout.firstRow && span.first <= layout.endRow) {
      const gy = layout.gridTop + span.first * cell - scrollTop;
      drawRowHairline(ctx, layout.gridLeft, layout.width, gy, theme, 'band');
    }
  }
  for (const span of layout.colGroups) {
    if (span.first > 0 && span.first >= layout.firstCol && span.first <= layout.endCol) {
      const gx = Math.floor(layout.gridLeft + span.first * cell - scrollLeft) + 0.5;
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
    const y = layout.gridTop + r * cell - scrollTop;
    if (hoveredCell?.row === r) {
      drawRowHover(ctx, 0, y, layout.gridLeft, cell, theme, 2);
    }
    let cursorX = layout.rowGroupWidth + MATRIX_PAD;
    if (layout.showFlags) {
      const flagState = resolveEntityFlag(entity, artProvider);
      drawFlagSlot(ctx, cursorX, Math.round(y + (cell - FLAG_SLOT_HEIGHT) / 2), flagState, theme);
      cursorX += FLAG_SLOT_WIDTH + 6;
    }
    if (layout.showNames) {
      const nameW = Math.max(16, layout.gridLeft - cursorX - MATRIX_PAD);
      drawNameSlot(ctx, entity.label, cursorX, y, nameW, cell, theme, {
        fontSize: theme.fontSize.small,
        maxLines: 1,
        color: entity.nameMissing ? theme.muted : theme.text,
      });
    } else if (!layout.showFlags) {
      const code = entity.shortLabel ?? entity.tag ?? entity.label;
      drawNameSlot(ctx, code, cursorX, y, Math.max(16, layout.gridLeft - cursorX - 4), cell, theme, {
        fontSize: theme.fontSize.small,
        maxLines: 1,
      });
    }
  }

  for (const span of layout.rowGroups) {
    const y0 = layout.gridTop + span.first * cell - scrollTop;
    const h = span.count * cell;
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
  ctx.rect(layout.gridLeft, layout.legendHeight, Math.max(0, layout.width - layout.gridLeft), layout.headerHeight);
  ctx.clip();
  ctx.fillStyle = theme.surface;
  ctx.fillRect(layout.gridLeft, layout.legendHeight, Math.max(0, layout.width - layout.gridLeft), layout.headerHeight);

  if (layout.colGroupHeight > 0) {
    const bandY = layout.legendHeight;
    ctx.fillStyle = tokens.headerBand;
    ctx.fillRect(layout.gridLeft, bandY, Math.max(0, layout.width - layout.gridLeft), layout.colGroupHeight);
    for (const span of layout.colGroups) {
      const x0 = layout.gridLeft + span.first * cell - scrollLeft;
      const w = span.count * cell;
      if (x0 + w < layout.gridLeft || x0 > layout.width) continue;
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
    drawRowHairline(ctx, layout.gridLeft, layout.width, bandY + layout.colGroupHeight - 1, theme, 'normal');
  }

  const colTop = layout.legendHeight + layout.colGroupHeight;
  const colBandH = layout.gridTop - colTop;
  for (let c = layout.firstCol; c < layout.endCol; c++) {
    const entity = model.columns[c]!;
    const x = layout.gridLeft + c * cell - scrollLeft;
    if (hoveredCell?.column === c) {
      drawRowHover(ctx, x, colTop, cell, colBandH, theme, 1);
    }
    if (layout.showFlags) {
      const fx = Math.round(x + (cell - FLAG_SLOT_WIDTH) / 2);
      const fy = Math.round(colTop + (colBandH - FLAG_SLOT_HEIGHT) / 2);
      drawFlagSlot(ctx, fx, fy, resolveEntityFlag(entity, artProvider), theme);
    } else {
      const code = entity.shortLabel ?? entity.tag ?? entity.label;
      drawNameSlot(ctx, code, x + 2, colTop, Math.max(8, cell - 4), colBandH, theme, {
        fontSize: 10,
        align: 'center',
        maxLines: 1,
      });
    }
  }
  ctx.restore();

  ctx.fillStyle = theme.surface;
  ctx.fillRect(0, layout.legendHeight, layout.gridLeft, layout.headerHeight);
  if (model.corner) {
    ctx.font = `${LABEL_WEIGHT} ${theme.fontSize.small}px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    const cornerW = Math.max(0, layout.gridLeft - MATRIX_PAD * 2);
    ctx.fillText(ellipsise(model.corner, cornerW, s => textWidth(ctx, s)), layout.rowGroupWidth + MATRIX_PAD, colTop + colBandH / 2);
  }
  drawRowHairline(ctx, 0, layout.width, layout.gridTop - 1, theme, 'strong');
  const gx = Math.floor(layout.gridLeft) - 0.5;
  ctx.strokeStyle = tokens.hairlineStrong;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(gx, layout.legendHeight);
  ctx.lineTo(gx, layout.height);
  ctx.stroke();

  ctx.restore();
  return { drew: true };
}
