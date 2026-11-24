import { LABEL_WEIGHT, VALUE_WEIGHT } from '../../charts/draw.ts';
import { ellipsise } from '../../charts/labels.ts';
import { textWidth, type Canvas2D } from '../../charts/textWidth.ts';
import { linearTicks } from '../../charts/valueAxis.ts';
import {
  barCornerRadius,
  drawArtSlot,
  drawBar,
  drawFlagSlot,
  drawLegendEntry,
  drawMissingMark,
  drawNameSlot,
  drawPartialTab,
  drawRowFocusRing,
  drawRowHairline,
  drawRowHover,
  drawRowSelected,
  drawScrollFade,
  drawSplitBar,
  resolveKitTokens,
  type ArtSlotState,
  type FlagSlotState,
} from '../../draw/kit.ts';
import { drawHatchRect } from '../valueCanvas.ts';
import { formatCompactScalar, formatValueState, notRecordedLabel } from '../valueFormat.ts';
import { mix, parseColor } from '../../shell/appearance/color.ts';
import type { FamilyTheme } from '../contract.ts';
import {
  BARS_PAD_X,
  BARS_REMOVE_BUTTON_SIZE,
  BARS_VALUE_GAP,
  visibleBarsRange,
  type BarsLayout,
} from './layout.ts';
import {
  formatBarsValueField,
  type BarsDataRow,
  type BarsModel,
  type BarsRowMedia,
  type BarsSegment,
} from './model.ts';

export interface BarsGlideOffset {
  readonly fromY: number;
  readonly startedAt: number;
}

export interface BarsIconLookup {
  get(url: string): ImageBitmap | null | undefined;
}

export interface DrawFamilyBarsInput {
  readonly model: BarsModel;
  readonly layout: BarsLayout;
  readonly theme: FamilyTheme;
  readonly scrollTop: number;
  readonly now: number;
  readonly glideDuration: number;
  readonly glide?: ReadonlyMap<string, BarsGlideOffset>;
  readonly icons?: BarsIconLookup;
  readonly inspectedKey?: string | null;
  readonly selectedKey?: string | null;
  readonly focusedKey?: string | null;
}

export interface BarsPaintStats {
  readonly paintedRows: number;
  readonly truncatedKeys: readonly string[];
}

export interface BarsHitResult {
  readonly kind: 'none' | 'row' | 'right-row' | 'remove-region' | 'add-region';
  readonly rowKey?: string;
  readonly segmentKey?: string;
  readonly side?: 'left' | 'right';
}

function easeOutCubic(t: number): number {
  const u = 1 - Math.max(0, Math.min(1, t));
  return 1 - u * u * u;
}

function paintedRowY(
  key: string,
  baseY: number,
  glide: ReadonlyMap<string, BarsGlideOffset> | undefined,
  now: number,
  glideDuration: number,
): number {
  if (!glide || glideDuration <= 0) return baseY;
  const entry = glide.get(key);
  if (!entry) return baseY;
  const progress = (now - entry.startedAt) / glideDuration;
  if (progress >= 1) return baseY;
  return entry.fromY + (baseY - entry.fromY) * easeOutCubic(progress);
}

function inkOnFill(fill: string): string {
  const rgb = parseColor(fill);
  if (!rgb) return 'rgba(255, 255, 255, 0.96)';
  const channel = (c: number) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  const lum = 0.2126 * channel(rgb.r) + 0.7152 * channel(rgb.g) + 0.0722 * channel(rgb.b);
  return lum > 0.32 ? 'rgba(17, 17, 17, 0.88)' : 'rgba(255, 255, 255, 0.96)';
}

function drawRowMedia(
  ctx: Canvas2D,
  x: number,
  middleY: number,
  width: number,
  height: number,
  media: BarsRowMedia | undefined,
  theme: FamilyTheme,
  icons?: BarsIconLookup,
): void {
  if (!media || media.kind === 'none' || width <= 0 || height <= 0) return;
  const y = Math.round(middleY - height / 2);
  if (media.kind === 'flag') {
    const bitmap = media.iconUrl ? icons?.get(media.iconUrl) : undefined;
    let state: FlagSlotState;
    if (media.state === 'ready' && bitmap) {
      state = { kind: 'ready', bitmap, width: bitmap.width, height: bitmap.height };
    } else if (media.state === 'ready' && media.iconUrl && bitmap === null) {
      state = { kind: 'failed' };
    } else if (media.state === 'ready' && media.iconUrl && bitmap === undefined) {
      state = { kind: 'pending' };
    } else if (media.state === 'missing') {
      state = { kind: 'missing', tag: media.tag };
    } else if (media.state === 'failed' || media.state === 'refused') {
      state = { kind: 'failed' };
    } else {
      state = { kind: 'pending' };
    }
    drawFlagSlot(ctx, x, y, state, theme, { width, height, dimmed: media.dimmed });
    return;
  }
  const bitmap = media.iconUrl ? icons?.get(media.iconUrl) : undefined;
  let state: ArtSlotState;
  if (media.state === 'ready' && bitmap) {
    state = { kind: 'ready', bitmap, width: bitmap.width, height: bitmap.height };
  } else if (media.state === 'ready' && media.iconUrl && bitmap === null) {
    state = { kind: 'failed', reason: media.reason };
  } else if (media.state === 'ready' && media.iconUrl && bitmap === undefined) {
    state = { kind: 'pending' };
  } else if (media.state === 'missing') {
    state = { kind: 'missing', resolvedName: media.resolvedName };
  } else if (media.state === 'refused') {
    state = { kind: 'refused', reason: media.reason };
  } else if (media.state === 'failed') {
    state = { kind: 'failed', reason: media.reason };
  } else if (media.state === 'none') {
    state = { kind: 'none' };
  } else {
    state = { kind: 'pending' };
  }
  drawArtSlot(ctx, x, y, width, height, state, theme, { radius: 2 });
}

function drawShareSegments(
  ctx: Canvas2D,
  x: number,
  y: number,
  width: number,
  height: number,
  segments: readonly BarsSegment[],
  theme: FamilyTheme,
  radius = 2.5,
): void {
  if (width <= 0 || height <= 0) return;
  const tokens = resolveKitTokens(theme);
  ctx.save();
  ctx.fillStyle = tokens.barTrack;
  ctx.beginPath();
  ctx.roundRect(x, y, width, height, radius);
  ctx.fill();
  ctx.beginPath();
  ctx.roundRect(x, y, width, height, radius);
  ctx.clip();

  const active = segments.filter(seg => seg.share > 0);
  const gap = active.length > 1 ? 1 : 0;
  const usable = Math.max(0, width - gap * Math.max(0, active.length - 1));
  let cursor = x;
  for (let i = 0; i < active.length; i++) {
    const seg = active[i]!;
    const segW = i === active.length - 1 ? Math.max(0, x + width - cursor) : usable * seg.share;
    if (segW <= 0) continue;
    if (seg.hatched) {
      drawHatchRect(ctx, cursor, y, segW, height, 0, theme);
    } else {
      ctx.fillStyle = seg.color;
      ctx.fillRect(cursor, y, segW, height);
    }
    if (!seg.hatched && segW >= 34) {
      const pctText = `${Math.round(seg.share * 100)}%`;
      const fullText = seg.inlineLabel ?? `${seg.label} ${pctText}`;
      ctx.font = `600 11px ${theme.font}`;
      ctx.fillStyle = inkOnFill(seg.color);
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      const pad = 6;
      const room = segW - pad * 2;
      if (textWidth(ctx, fullText) <= room) {
        ctx.fillText(fullText, cursor + pad, y + height / 2);
      } else if (textWidth(ctx, pctText) <= room) {
        ctx.fillText(pctText, cursor + pad, y + height / 2);
      }
    }
    cursor += segW + gap;
  }
  ctx.restore();
}

function drawHeader(ctx: Canvas2D, model: BarsModel, layout: BarsLayout, theme: FamilyTheme): void {
  if (layout.headerHeight <= 0) return;
  const middleY = layout.headerTop + layout.headerHeight / 2;
  ctx.save();
  ctx.textBaseline = 'middle';
  if (model.form === 'two-sided' && layout.twoSidedLayout === 'paired') {
    ctx.font = `500 ${theme.fontSize.body}px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    if (model.leftHeader) {
      ctx.textAlign = 'right';
      ctx.fillText(model.leftHeader, layout.centerX - 10, middleY);
    }
    if (model.rightHeader) {
      ctx.textAlign = 'left';
      ctx.fillText(model.rightHeader, layout.centerX + 10, middleY);
    }
  } else {
    if (model.caption) {
      ctx.font = `500 ${theme.fontSize.body}px ${theme.font}`;
      ctx.fillStyle = theme.text;
      ctx.textAlign = 'left';
      ctx.fillText(ellipsise(model.caption, layout.width - BARS_PAD_X * 2, s => textWidth(ctx, s)), BARS_PAD_X, middleY);
    }
    if (model.captionRight) {
      ctx.font = `${LABEL_WEIGHT} ${theme.fontSize.small}px ${theme.font}`;
      ctx.fillStyle = theme.muted;
      ctx.textAlign = 'right';
      ctx.fillText(model.captionRight, layout.width - BARS_PAD_X, middleY);
    }
  }
  ctx.restore();
}

function drawAxis(ctx: Canvas2D, model: BarsModel, layout: BarsLayout, theme: FamilyTheme): void {
  if (!layout.showAxis || layout.axisHeight <= 0) return;
  const tokens = resolveKitTokens(theme);
  const ticks = linearTicks(0, layout.scaleMax, 4);
  const y = layout.axisTop + layout.axisHeight / 2;
  ctx.save();
  ctx.font = `${LABEL_WEIGHT} 11px ${theme.font}`;
  ctx.textBaseline = 'middle';
  if (model.scaleUnit) {
    ctx.fillStyle = theme.muted;
    ctx.textAlign = 'left';
    ctx.fillText(ellipsise(model.scaleUnit, Math.max(0, layout.plotLeft - BARS_PAD_X - 6), s => textWidth(ctx, s)), BARS_PAD_X, y);
  }
  for (let i = 0; i < ticks.length; i++) {
    const tick = ticks[i]!;
    if (tick < 0 || tick > layout.scaleMax) continue;
    const tx = layout.plotLeft + (tick / layout.scaleMax) * layout.plotWidth;
    ctx.strokeStyle = tokens.gridLine;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(Math.floor(tx) + 0.5, layout.bodyTop);
    ctx.lineTo(Math.floor(tx) + 0.5, layout.bodyBottom);
    ctx.stroke();

    ctx.fillStyle = theme.muted;
    ctx.textAlign = i === 0 ? 'left' : i === ticks.length - 1 ? 'right' : 'center';
    const text = formatCompactScalar(tick, model.scaleFormat).text;
    ctx.fillText(text, tx, y);
  }
  ctx.restore();
}

function drawLegend(ctx: Canvas2D, model: BarsModel, layout: BarsLayout, theme: FamilyTheme): void {
  if (layout.legendHeight <= 0 || !model.legend?.length) return;
  const middleY = layout.legendTop + layout.legendHeight / 2;
  let x = BARS_PAD_X;
  const rightLimit = layout.width - BARS_PAD_X;
  for (let i = 0; i < model.legend.length; i++) {
    const entry = model.legend[i]!;
    const room = rightLimit - x;
    if (room < 32) break;
    const text = entry.share ? `${entry.label} ${entry.share}` : entry.value ? `${entry.label} ${entry.value}` : entry.label;
    const drawn = drawLegendEntry(ctx, x, middleY, text, entry.color, theme, { maxWidth: room });
    x += drawn.width + 14;
  }
}

export function drawFamilyBars(ctx: Canvas2D, input: DrawFamilyBarsInput): BarsPaintStats {
  const { model, layout, theme, scrollTop, now, glideDuration, glide, icons, inspectedKey, selectedKey, focusedKey } = input;
  const tokens = resolveKitTokens(theme);
  ctx.clearRect(0, 0, layout.width, layout.height);

  drawHeader(ctx, model, layout, theme);
  if (layout.showAxis) drawAxis(ctx, model, layout, theme);

  const { first, last } = visibleBarsRange(layout, scrollTop);
  const truncatedKeys: string[] = [];
  let paintedRows = 0;

  ctx.save();
  ctx.beginPath();
  ctx.rect(0, layout.bodyTop, layout.width, layout.bodyHeight);
  ctx.clip();

  if (model.form === 'net' || model.form === 'two-sided') {
    const px = Math.floor(layout.centerX) + 0.5;
    ctx.save();
    ctx.strokeStyle = model.form === 'net' ? tokens.zeroLine : tokens.hairlineStrong;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(px, layout.bodyTop);
    ctx.lineTo(px, layout.bodyBottom);
    ctx.stroke();
    ctx.restore();
  }

  for (let i = first; i < last; i++) {
    const item = model.rows[i]!;
    const baseY = layout.bodyTop + layout.rowTops[i]! - scrollTop;
    const rowH = layout.rowHeights[i]!;
    const y = paintedRowY(item.key, baseY, glide, now, glideDuration);
    const middleY = y + rowH / 2;
    paintedRows++;

    if (item.kind === 'section') {
      ctx.save();
      ctx.font = `${LABEL_WEIGHT} 11px ${theme.font}`;
      ctx.fillStyle = theme.muted;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillText(item.label, BARS_PAD_X, middleY);
      ctx.restore();
      continue;
    }

    if (item.kind === 'add-region') {
      const hovered = inspectedKey === item.key;
      if (hovered) drawRowHover(ctx, 0, y, layout.width, rowH, theme);
      if (focusedKey === item.key) drawRowFocusRing(ctx, 0, y, layout.width, rowH, theme);
      ctx.save();
      ctx.font = `500 ${theme.fontSize.body}px ${theme.font}`;
      ctx.fillStyle = theme.accent;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillText(`+ ${item.label}`, layout.nameX, middleY);
      ctx.strokeStyle = tokens.hairlineStrong;
      ctx.lineWidth = 1;
      ctx.setLineDash([4, 4]);
      const py = Math.floor(middleY) + 0.5;
      ctx.beginPath();
      ctx.moveTo(layout.plotLeft, py);
      ctx.lineTo(layout.plotRight, py);
      ctx.stroke();
      ctx.setLineDash([]);
      ctx.restore();
      continue;
    }

    const row: BarsDataRow = item;
    const isHovered = inspectedKey === row.key || (row.rightSide && inspectedKey === row.rightSide.key);
    const isSelected = selectedKey === row.key || (row.rightSide && selectedKey === row.rightSide.key);
    const isFocused = focusedKey === row.key || (row.rightSide && focusedKey === row.rightSide.key);

    if (isSelected) drawRowSelected(ctx, 0, y, layout.width, rowH, theme);
    else if (isHovered) drawRowHover(ctx, 0, y, layout.width, rowH, theme);
    if (isFocused) drawRowFocusRing(ctx, 0, y, layout.width, rowH, theme);
    if (model.form === 'net' && i < model.rows.length - 1) {
      drawRowHairline(ctx, BARS_PAD_X, layout.width - BARS_PAD_X, y + rowH - 1, theme, 'normal');
    }

    drawRowMedia(ctx, BARS_PAD_X, middleY, layout.mediaWidth, layout.mediaHeight, row.media, theme, icons);

    if (layout.nameWidth > 0) {
      const reserveRemove = row.removable && (isHovered || isFocused) ? BARS_REMOVE_BUTTON_SIZE + 4 : 0;
      const nameColor = row.labelMissing || row.muted || row.group === 'auto' ? theme.muted : theme.text;
      const fitted = drawNameSlot(
        ctx,
        row.labelMissing ? notRecordedLabel('Name') : row.label,
        layout.nameX,
        y,
        Math.max(12, layout.nameWidth - reserveRemove),
        rowH,
        theme,
        { color: nameColor, weight: model.form === 'net' ? 500 : LABEL_WEIGHT },
      );
      if (fitted.truncated) truncatedKeys.push(row.key);
      if (row.removable && (isHovered || isFocused)) {
        const bx = layout.nameX + layout.nameWidth - BARS_REMOVE_BUTTON_SIZE;
        const by = Math.round(middleY - BARS_REMOVE_BUTTON_SIZE / 2);
        ctx.save();
        ctx.fillStyle = tokens.barMuted;
        ctx.beginPath();
        ctx.roundRect(bx, by, BARS_REMOVE_BUTTON_SIZE, BARS_REMOVE_BUTTON_SIZE, 4);
        ctx.fill();
        ctx.fillStyle = theme.text;
        ctx.font = `500 11px ${theme.font}`;
        ctx.textAlign = 'center';
        ctx.textBaseline = 'middle';
        ctx.fillText('\u00d7', bx + BARS_REMOVE_BUTTON_SIZE / 2, middleY);
        ctx.restore();
      }
    }

    const barY = Math.round(middleY - layout.barHeight / 2);
    const barColor = row.labelMissing ? theme.grey : (row.color ?? theme.accent);

    if (model.form === 'net') {
      const formatted = row.value ? formatBarsValueField(row.value) : null;
      const halfSpan = Math.max(1, (layout.plotRight - layout.plotLeft) / 2);
      if (formatted && (formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable')) {
        drawMissingMark(ctx, layout.plotLeft, middleY, layout.plotWidth, theme, {
          figure: row.value?.figure,
          notApplicable: formatted.kind === 'notApplicable',
        });
      } else if (row.numeric !== undefined && row.numeric !== null && row.numeric !== 0) {
        const rawW = (Math.abs(row.numeric) / layout.scaleMax) * halfSpan;
        const barW = Math.min(halfSpan, Math.max(2, rawW));
        if (row.numeric > 0) {
          drawBar(ctx, layout.centerX, barY, barW, layout.barHeight, barColor, 'right');
        } else {
          drawBar(ctx, layout.centerX - barW, barY, barW, layout.barHeight, tokens.shortage, 'left');
        }
      }

      if (layout.showReadout && formatted) {
        const rightEdge = layout.readoutX + layout.readoutWidth;
        if (formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable') {
          drawMissingMark(ctx, layout.readoutX, middleY, layout.readoutWidth, theme, {
            align: 'right',
            notApplicable: formatted.kind === 'notApplicable',
          });
        } else {
          ctx.save();
          ctx.textAlign = 'right';
          ctx.textBaseline = 'middle';
          ctx.font = `600 13px ${theme.font}`;
          ctx.fillStyle = row.value?.tone === 'shortage' || (row.numeric ?? 0) < 0
            ? tokens.shortage
            : (row.numeric ?? 0) === 0
              ? theme.muted
              : theme.text;
          ctx.fillText(ellipsise(formatted.text, layout.readoutWidth, s => textWidth(ctx, s)), rightEdge, middleY - 7);
          const sub = row.value?.caption ?? ((row.numeric ?? 0) < 0 ? 'Short a day' : (row.numeric ?? 0) > 0 ? 'Surplus a day' : 'Balanced');
          ctx.font = `${LABEL_WEIGHT} 11px ${theme.font}`;
          ctx.fillStyle = theme.muted;
          ctx.fillText(ellipsise(sub, layout.readoutWidth, s => textWidth(ctx, s)), rightEdge, middleY + 8);
          ctx.restore();
        }
      }

      if (layout.showPair && row.pair) {
        const topFmt = formatValueState(row.pair.topValue, row.pair.format);
        const botFmt = formatValueState(row.pair.bottomValue, row.pair.format);
        const topVal = topFmt.kind === 'notRecorded' || topFmt.kind === 'notApplicable' ? topFmt.label : topFmt.text;
        const botVal = botFmt.kind === 'notRecorded' || botFmt.kind === 'notApplicable' ? botFmt.label : botFmt.text;
        const rightEdge = layout.pairX + layout.pairWidth;
        ctx.save();
        ctx.textBaseline = 'middle';
        ctx.font = `${LABEL_WEIGHT} 11px ${theme.font}`;
        ctx.fillStyle = theme.muted;
        ctx.textAlign = 'left';
        ctx.fillText(row.pair.topLabel, layout.pairX, middleY - 7);
        ctx.fillText(row.pair.bottomLabel, layout.pairX, middleY + 8);
        ctx.font = `${VALUE_WEIGHT} 12px ${theme.font}`;
        ctx.fillStyle = theme.text;
        ctx.textAlign = 'right';
        ctx.fillText(topVal, rightEdge, middleY - 7);
        ctx.fillText(botVal, rightEdge, middleY + 8);
        ctx.restore();
      }
      continue;
    }

    if (model.form === 'share' || model.form === 'stacked') {
      const valueMissing = row.value && (row.value.value.kind === 'notRecorded' || row.value.value.kind === 'notApplicable');
      if (valueMissing || (!row.segments?.length && row.value?.value.kind === 'notRecorded')) {
        drawHatchRect(ctx, layout.plotLeft, barY, layout.plotWidth, layout.barHeight, 2.5, theme);
        const msg = row.value?.value.kind === 'notApplicable' ? '\u2014' : notRecordedLabel(row.value?.figure);
        ctx.save();
        ctx.font = `${LABEL_WEIGHT} 11px ${theme.font}`;
        ctx.fillStyle = theme.muted;
        ctx.textAlign = 'center';
        ctx.textBaseline = 'middle';
        ctx.fillText(ellipsise(msg, Math.max(0, layout.plotWidth - 8), s => textWidth(ctx, s)), layout.plotLeft + layout.plotWidth / 2, middleY);
        ctx.restore();
      } else if (row.segments?.length) {
        drawShareSegments(ctx, layout.plotLeft, barY, layout.plotWidth, layout.barHeight, row.segments, theme, 2.5);
      } else {
        ctx.save();
        ctx.fillStyle = tokens.barTrack;
        ctx.beginPath();
        ctx.roundRect(layout.plotLeft, barY, layout.plotWidth, layout.barHeight, 2.5);
        ctx.fill();
        ctx.restore();
      }
      if (layout.showReadout && row.readout) {
        ctx.save();
        ctx.font = `${LABEL_WEIGHT} ${theme.fontSize.body}px ${theme.font}`;
        ctx.fillStyle = row.readout.tone === 'shortage' ? tokens.shortage : theme.muted;
        ctx.textAlign = 'right';
        ctx.textBaseline = 'middle';
        ctx.fillText(
          ellipsise(row.readout.primary, layout.readoutWidth, s => textWidth(ctx, s)),
          layout.readoutX + layout.readoutWidth,
          middleY,
        );
        ctx.restore();
      }
      continue;
    }

    if (model.form === 'two-sided' && layout.twoSidedLayout === 'split-sides') {
      if (row.value) {
        const leftFmt = formatBarsValueField(row.value);
        const leftSpan = Math.max(1, layout.plotRight - layout.plotLeft);
        if (leftFmt.kind === 'notRecorded' || leftFmt.kind === 'notApplicable') {
          drawMissingMark(ctx, layout.plotLeft, middleY, leftSpan, theme, { align: 'right', notApplicable: leftFmt.kind === 'notApplicable' });
        } else {
          if (row.numeric && row.numeric > 0) {
            const w = Math.min(leftSpan, Math.max(2, (row.numeric / layout.scaleLeftMax) * leftSpan));
            drawBar(ctx, layout.plotRight - w, barY, w, layout.barHeight, barColor, 'left');
          }
          ctx.save();
          ctx.font = `500 ${theme.fontSize.body}px ${theme.font}`;
          ctx.fillStyle = leftFmt.kind === 'zero' ? theme.muted : theme.text;
          ctx.textAlign = 'right';
          ctx.textBaseline = 'middle';
          ctx.fillText(leftFmt.text, layout.centerX - 18, middleY);
          ctx.restore();
        }
      }
      if (row.rightSide) {
        const right = row.rightSide;
        const rightFmt = formatBarsValueField(right.value);
        const rightSpan = Math.max(1, layout.rightPlotRight - layout.rightPlotLeft);
        const rightColor = right.labelMissing ? theme.grey : (right.color ?? theme.accent);
        if (rightFmt.kind === 'notRecorded' || rightFmt.kind === 'notApplicable') {
          drawMissingMark(ctx, layout.rightPlotLeft, middleY, rightSpan, theme, { align: 'left', notApplicable: rightFmt.kind === 'notApplicable' });
        } else {
          ctx.save();
          ctx.font = `500 ${theme.fontSize.body}px ${theme.font}`;
          ctx.fillStyle = rightFmt.kind === 'zero' ? theme.muted : theme.text;
          ctx.textAlign = 'left';
          ctx.textBaseline = 'middle';
          ctx.fillText(rightFmt.text, layout.centerX + 18, middleY);
          ctx.restore();
          if (right.numeric && right.numeric > 0) {
            const w = Math.min(rightSpan, Math.max(2, (right.numeric / layout.scaleRightMax) * rightSpan));
            drawBar(ctx, layout.rightPlotLeft, barY, w, layout.barHeight, rightColor, 'right');
          }
        }
        const rightName = drawNameSlot(
          ctx,
          right.labelMissing ? notRecordedLabel('Name') : right.label,
          layout.rightNameX,
          y,
          layout.rightNameWidth,
          rowH,
          theme,
          { align: 'right', color: right.labelMissing ? theme.muted : theme.text },
        );
        if (rightName.truncated) truncatedKeys.push(right.key);
        drawRowMedia(ctx, layout.width - BARS_PAD_X - layout.mediaWidth, middleY, layout.mediaWidth, layout.mediaHeight, right.media, theme, icons);
      }
      continue;
    }

    if (model.form === 'two-sided') {
      const leftAvail = Math.max(1, layout.centerX - layout.plotLeft - layout.valueLeftGutter);
      const rightAvail = Math.max(1, layout.plotRight - layout.centerX - layout.valueRightGutter);
      const leftFill = row.left?.color ?? mix(barColor, theme.surface, 0.42);
      const rightFill = row.right?.color ?? barColor;

      if (row.left) {
        const leftFmt = formatBarsValueField(row.left.value);
        if (leftFmt.kind === 'notRecorded' || leftFmt.kind === 'notApplicable') {
          drawMissingMark(ctx, layout.plotLeft, middleY, Math.max(12, layout.centerX - layout.plotLeft - 6), theme, {
            align: 'right',
            notApplicable: leftFmt.kind === 'notApplicable',
          });
        } else {
          const w = row.left.numeric && row.left.numeric > 0
            ? Math.min(leftAvail, Math.max(2, (row.left.numeric / layout.scaleLeftMax) * leftAvail))
            : 0;
          if (w > 0) drawBar(ctx, layout.centerX - w, barY, w, layout.barHeight, leftFill, 'left');
          ctx.save();
          ctx.font = `500 ${theme.fontSize.body}px ${theme.font}`;
          ctx.fillStyle = leftFmt.kind === 'zero' ? theme.muted : theme.text;
          ctx.textAlign = 'right';
          ctx.textBaseline = 'middle';
          ctx.fillText(leftFmt.text, layout.centerX - w - BARS_VALUE_GAP, middleY);
          ctx.restore();
        }
      }
      if (row.right) {
        const rightFmt = formatBarsValueField(row.right.value);
        if (rightFmt.kind === 'notRecorded' || rightFmt.kind === 'notApplicable') {
          drawMissingMark(ctx, layout.centerX + 6, middleY, Math.max(12, layout.plotRight - layout.centerX - 6), theme, {
            align: 'left',
            notApplicable: rightFmt.kind === 'notApplicable',
          });
        } else {
          const w = row.right.numeric && row.right.numeric > 0
            ? Math.min(rightAvail, Math.max(2, (row.right.numeric / layout.scaleRightMax) * rightAvail))
            : 0;
          if (w > 0) drawBar(ctx, layout.centerX, barY, w, layout.barHeight, rightFill, 'right');
          ctx.save();
          ctx.font = `500 ${theme.fontSize.body}px ${theme.font}`;
          ctx.fillStyle = rightFmt.kind === 'zero' ? theme.muted : theme.text;
          ctx.textAlign = 'left';
          ctx.textBaseline = 'middle';
          ctx.fillText(rightFmt.text, layout.centerX + w + BARS_VALUE_GAP, middleY);
          ctx.restore();
        }
      }
      continue;
    }

    const formatted = row.value ? formatBarsValueField(row.value) : null;
    if (formatted && (formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable')) {
      drawMissingMark(ctx, layout.plotLeft, middleY, layout.plotWidth, theme, {
        figure: row.value?.figure,
        notApplicable: formatted.kind === 'notApplicable',
      });
      continue;
    }

    let drawnEndX = layout.plotLeft;
    if (row.target && row.target.numeric > 0) {
      const targetW = Math.min(layout.plotWidth, Math.max(2, (row.target.numeric / layout.scaleMax) * layout.plotWidth));
      const delivered = Math.max(0, row.numeric ?? 0);
      const deliveredW = delivered > 0
        ? Math.min(targetW, Math.max(2, (delivered / layout.scaleMax) * layout.plotWidth))
        : 0;
      if (deliveredW > 0) {
        drawBar(ctx, layout.plotLeft, barY, deliveredW, layout.barHeight, barColor, 'right');
      }
      if (targetW > deliveredW + 1) {
        const gapX = layout.plotLeft + deliveredW + (deliveredW > 0 ? 1 : 0);
        const gapW = Math.max(2, targetW - deliveredW - (deliveredW > 0 ? 1 : 0));
        const r = barCornerRadius(layout.barHeight);
        ctx.save();
        ctx.fillStyle = tokens.shortage;
        ctx.globalAlpha = 0.16;
        ctx.beginPath();
        ctx.roundRect(gapX, barY, gapW, layout.barHeight, [0, r, r, 0]);
        ctx.fill();
        ctx.globalAlpha = 0.9;
        ctx.strokeStyle = tokens.shortage;
        ctx.lineWidth = 1;
        ctx.setLineDash([2.5, 2]);
        ctx.beginPath();
        ctx.roundRect(gapX + 0.5, barY + 0.5, Math.max(0, gapW - 1), Math.max(0, layout.barHeight - 1), [0, r, r, 0]);
        ctx.stroke();
        ctx.setLineDash([]);
        ctx.restore();
      }
      drawnEndX = layout.plotLeft + targetW;
    } else if (row.segments?.length) {
      const totalNumeric = Math.max(0, row.numeric ?? 0);
      const totalBarW = totalNumeric > 0 ? Math.min(layout.plotWidth, Math.max(2, (totalNumeric / layout.scaleMax) * layout.plotWidth)) : 0;
      if (totalBarW > 0) {
        drawSplitBar(
          ctx,
          layout.plotLeft,
          barY,
          totalBarW,
          layout.barHeight,
          row.segments.map(seg => ({ fraction: seg.share, fill: seg.color, hatched: seg.hatched })),
          theme,
        );
      }
      drawnEndX = layout.plotLeft + totalBarW;
    } else if (row.numeric !== undefined && row.numeric !== null && row.numeric > 0) {
      const barW = Math.min(layout.plotWidth, Math.max(2, (row.numeric / layout.scaleMax) * layout.plotWidth));
      drawBar(ctx, layout.plotLeft, barY, barW, layout.barHeight, barColor, 'right');
      drawnEndX = layout.plotLeft + barW;
    }

    if (layout.showReadout && row.readout) {
      const rightEdge = layout.readoutX + layout.readoutWidth;
      ctx.save();
      ctx.textBaseline = 'middle';
      ctx.textAlign = 'right';
      let cursorRight = rightEdge;
      if (row.readout.shortfall) {
        ctx.font = `600 ${theme.fontSize.body}px ${theme.font}`;
        ctx.fillStyle = tokens.shortage;
        ctx.fillText(row.readout.shortfall, cursorRight, middleY);
        cursorRight -= textWidth(ctx, row.readout.shortfall) + 8;
      }
      ctx.font = `500 ${theme.fontSize.body}px ${theme.font}`;
      ctx.fillStyle = row.readout.tone === 'shortage' ? tokens.shortage : theme.text;
      const primaryW = textWidth(ctx, row.readout.primary);
      ctx.fillText(ellipsise(row.readout.primary, Math.max(12, cursorRight - layout.readoutX), s => textWidth(ctx, s)), cursorRight, middleY);
      if (row.readout.iconUrl || row.readout.iconMissing) {
        const iconSize = 16;
        const iconX = cursorRight - primaryW - iconSize - 5;
        if (iconX >= layout.readoutX) {
          const bmp = row.readout.iconUrl ? icons?.get(row.readout.iconUrl) : undefined;
          const artState: ArtSlotState = row.readout.iconMissing
            ? { kind: 'missing' }
            : bmp
              ? { kind: 'ready', bitmap: bmp, width: bmp.width, height: bmp.height }
              : { kind: 'pending' };
          drawArtSlot(ctx, iconX, Math.round(middleY - iconSize / 2), iconSize, iconSize, artState, theme, { radius: 2 });
        }
      }
      ctx.restore();
    } else if (formatted) {
      ctx.save();
      ctx.font = `500 ${theme.fontSize.body}px ${theme.font}`;
      ctx.textBaseline = 'middle';
      const labelW = textWidth(ctx, formatted.text);
      const fitsOutside = !layout.valueInsideBar && drawnEndX + BARS_VALUE_GAP + labelW <= layout.width - BARS_PAD_X;
      if (fitsOutside || drawnEndX - layout.plotLeft < labelW + 12) {
        ctx.fillStyle = formatted.kind === 'zero' ? theme.muted : theme.text;
        ctx.textAlign = 'left';
        ctx.fillText(formatted.text, drawnEndX + BARS_VALUE_GAP, middleY);
        if (formatted.kind === 'partial') {
          drawPartialTab(ctx, drawnEndX + BARS_VALUE_GAP + labelW + 5, middleY, theme);
        }
      } else {
        ctx.fillStyle = inkOnFill(barColor);
        ctx.textAlign = 'right';
        ctx.fillText(formatted.text, drawnEndX - 6, middleY);
      }
      ctx.restore();
    }
  }

  if (scrollTop > 0) {
    drawScrollFade(ctx, 0, layout.bodyTop, layout.width, 18, theme, 'top');
  }
  if (scrollTop + layout.bodyHeight < layout.totalHeight - 1) {
    drawScrollFade(ctx, 0, layout.bodyBottom - 24, layout.width, 24, theme, 'bottom');
  }
  ctx.restore();

  if (layout.legendHeight > 0) drawLegend(ctx, model, layout, theme);

  return { paintedRows, truncatedKeys };
}

export function hitFamilyBars(
  layout: BarsLayout,
  model: BarsModel,
  point: { readonly x: number; readonly y: number },
  scrollTop: number,
  glide?: ReadonlyMap<string, BarsGlideOffset>,
  now = 0,
  glideDuration = 0,
): BarsHitResult {
  if (point.y < layout.bodyTop || point.y > layout.bodyBottom) return { kind: 'none' };
  const { first, last } = visibleBarsRange(layout, scrollTop);
  for (let i = first; i < last; i++) {
    const item = model.rows[i]!;
    const baseY = layout.bodyTop + layout.rowTops[i]! - scrollTop;
    const rowH = layout.rowHeights[i]!;
    const y = paintedRowY(item.key, baseY, glide, now, glideDuration);
    if (point.y < y || point.y >= y + rowH) continue;
    if (item.kind === 'section') return { kind: 'none' };
    if (item.kind === 'add-region') return { kind: 'add-region', rowKey: item.key };
    if (item.removable) {
      const bx = layout.nameX + layout.nameWidth - BARS_REMOVE_BUTTON_SIZE;
      if (point.x >= bx && point.x <= layout.nameX + layout.nameWidth) {
        return { kind: 'remove-region', rowKey: item.key };
      }
    }
    if (model.form === 'two-sided' && layout.twoSidedLayout === 'split-sides' && item.rightSide && point.x >= layout.centerX) {
      return { kind: 'right-row', rowKey: item.rightSide.key, side: 'right' };
    }
    let segmentKey: string | undefined;
    if ((model.form === 'share' || model.form === 'stacked') && item.segments?.length && point.x >= layout.plotLeft && point.x <= layout.plotRight) {
      const active = item.segments.filter(s => s.share > 0);
      const rel = (point.x - layout.plotLeft) / Math.max(1, layout.plotWidth);
      let acc = 0;
      for (let s = 0; s < active.length; s++) {
        acc += active[s]!.share;
        if (rel <= acc || s === active.length - 1) {
          segmentKey = active[s]!.key;
          break;
        }
      }
    }
    return {
      kind: 'row',
      rowKey: item.key,
      ...(segmentKey ? { segmentKey } : {}),
      ...(model.form === 'two-sided' ? { side: point.x < layout.centerX ? 'left' : 'right' } : {}),
    };
  }
  return { kind: 'none' };
}

export function collectVisibleBarsIcons(model: BarsModel, layout: BarsLayout, scrollTop: number): string[] {
  const { first, last } = visibleBarsRange(layout, scrollTop);
  const urls = new Set<string>();
  for (let i = first; i < last; i++) {
    const item = model.rows[i]!;
    if (item.kind !== 'data') continue;
    if (item.media?.iconUrl) urls.add(item.media.iconUrl);
    if (item.rightSide?.media?.iconUrl) urls.add(item.rightSide.media.iconUrl);
    if (item.readout?.iconUrl) urls.add(item.readout.iconUrl);
    if (item.target?.iconUrl) urls.add(item.target.iconUrl);
  }
  return [...urls];
}
