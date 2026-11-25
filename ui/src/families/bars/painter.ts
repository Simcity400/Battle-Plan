import { LABEL_WEIGHT, VALUE_WEIGHT } from '../../charts/draw.ts';
import { readableColour } from '../../charts/palette.ts';
import { ellipsise } from '../../charts/labels.ts';
import { textWidth, type Canvas2D } from '../../charts/textWidth.ts';
import { linearTicks } from '../../charts/valueAxis.ts';
import {
  barCornerRadius,
  contrastInkForFill,
  drawShareBar,
  drawBar,
  drawLegendStrip,
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
  drawAddRow,
  drawRowGrip,
  drawRowRemoveButton,
  drawRowSectionLabel,
  ROW_REMOVE_SIZE,
  rowGripBox,
  rowRemoveBox,
} from '../../draw/kit.ts';
import { drawMediaSlot } from '../art/adapters.ts';
import type { LaneArtCache } from '../art/laneCache.ts';
import { collectArtUrls, type ArtUrlInput } from '../art/provider.ts';
import { drawHatchRect } from '../valueCanvas.ts';
import { formatCompactScalar, formatShare, formatValueState, notRecordedLabel } from '../valueFormat.ts';
import { mix } from '../../shell/appearance/color.ts';
import type { FamilyTheme } from '../contract.ts';
import {
  BARS_PAD_X,
  BARS_VALUE_GAP,
  visibleBarsRange,
  type BarsLayout,
} from './layout.ts';
import {
  formatBarsValueField,
  type BarsDataRow,
  type BarsModel,
  type BarsValueField,
} from './model.ts';
import { drawCaptionSides, drawPickList } from './pickList.ts';

export interface BarsGlideOffset {
  readonly fromY: number;
  readonly startedAt: number;
}


export interface DrawFamilyBarsInput {
  readonly model: BarsModel;
  readonly layout: BarsLayout;
  readonly theme: FamilyTheme;
  readonly scrollTop: number;
  readonly now: number;
  readonly glideDuration: number;
  readonly glide?: ReadonlyMap<string, BarsGlideOffset>;
  readonly artCache?: LaneArtCache;
  readonly inspectedKey?: string | null;
  readonly selectedKey?: string | null;
  readonly focusedKey?: string | null;
}

export interface BarsPaintStats {
  readonly paintedRows: number;
  readonly truncatedKeys: readonly string[];
}

export interface BarsHitResult {
  readonly kind: 'none' | 'row' | 'right-row' | 'remove-region' | 'grip-region' | 'add-region';
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

function drawStackedPair(ctx: Canvas2D, row: BarsDataRow, layout: BarsLayout, theme: FamilyTheme, fill: string, middleY: number): void {
  const span = layout.plotWidth;
  const h = layout.barHeight;
  const solidY = Math.round(middleY - h - 1.5);
  const outlineY = Math.round(middleY + 1.5);
  const sides = [
    { measure: row.right, y: solidY, solid: true },
    { measure: row.left, y: outlineY, solid: false },
  ];
  const missing = sides.filter(side => side.measure && !measured(side.measure.value));
  if (missing.length === 2) {
    const kind = formatBarsValueField(row.right!.value).kind;
    drawMissingMark(ctx, layout.plotLeft, middleY, span, theme, { notApplicable: kind === 'notApplicable' });
  } else {
    for (const side of sides) {
      if (!side.measure) continue;
      if (!measured(side.measure.value)) {
        drawHatchRect(ctx, layout.plotLeft, side.y, Math.min(span, 10), h, 0, theme);
        continue;
      }
      const numeric = side.measure.numeric ?? 0;
      if (numeric <= 0) continue;
      const w = Math.min(span, Math.max(2, (numeric / layout.scaleMax) * span));
      if (side.solid) {
        drawBar(ctx, layout.plotLeft, side.y, w, h, fill, 'right');
      } else {
        ctx.save();
        ctx.strokeStyle = fill;
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.roundRect(layout.plotLeft + 0.5, side.y + 0.5, Math.max(1, w - 1), h - 1, [0, 2, 2, 0]);
        ctx.stroke();
        ctx.restore();
      }
    }
  }
  if (layout.showReadout && row.readout) {
    ctx.save();
    ctx.font = `500 ${theme.fontSize.body}px ${theme.font}`;
    ctx.fillStyle = theme.text;
    ctx.textAlign = 'right';
    ctx.textBaseline = 'middle';
    ctx.fillText(ellipsise(row.readout.primary, layout.readoutWidth, t => textWidth(ctx, t)), layout.readoutX + layout.readoutWidth, middleY);
    ctx.restore();
  }
}

function measured(field: BarsValueField): boolean {
  const kind = formatBarsValueField(field).kind;
  return kind !== 'notRecorded' && kind !== 'notApplicable';
}

function drawHeader(ctx: Canvas2D, model: BarsModel, layout: BarsLayout, theme: FamilyTheme): void {
  if (layout.headerHeight <= 0) return;
  const middleY = layout.headerTop + layout.headerHeight / 2;
  ctx.save();
  ctx.textBaseline = 'middle';
  if (model.form === 'two-sided' && layout.twoSidedLayout === 'stacked-pair') {
    const legend = model.legend ?? [];
    if (legend.length > 0) {
      drawLegendStrip(ctx, layout.nameX, layout.headerTop, Math.max(0, layout.width - layout.nameX - BARS_PAD_X), layout.headerHeight, {
        entries: {
          length: legend.length,
          at: at => ({
            label: legend[at]!.label,
            color: theme.text,
            ...(at > 0 ? { outlined: true } : {}),
            muted: true,
          }),
        },
        entryGap: 18,
      }, theme);
    }
  } else if (model.form === 'two-sided' && layout.twoSidedLayout === 'paired') {
    ctx.font = `500 ${theme.fontSize.body}px ${theme.font}`;
    ctx.fillStyle = theme.text;
    ctx.textAlign = 'center';
    const leftMid = (layout.plotLeft + layout.centerX) / 2;
    const rightMid = (layout.centerX + layout.plotRight) / 2;
    if (model.leftHeader) {
      ctx.fillText(ellipsise(model.leftHeader, layout.centerX - layout.plotLeft - 8, s => textWidth(ctx, s)), leftMid, middleY);
    }
    if (model.rightHeader) {
      ctx.fillText(ellipsise(model.rightHeader, layout.plotRight - layout.centerX - 8, s => textWidth(ctx, s)), rightMid, middleY);
    }
    if (layout.showReadout && model.readoutHeader) {
      ctx.fillStyle = theme.muted;
      ctx.textAlign = 'right';
      ctx.fillText(model.readoutHeader, layout.readoutX + layout.readoutWidth, middleY);
    }
  } else {
    if (model.caption) {
      ctx.font = `500 ${theme.fontSize.body}px ${theme.font}`;
      ctx.fillStyle = theme.text;
      ctx.textAlign = 'left';
      ctx.fillText(ellipsise(model.caption, layout.width - layout.bodyLeft - BARS_PAD_X * 2, s => textWidth(ctx, s)), layout.bodyLeft + BARS_PAD_X, middleY);
    }
    if (model.captionRight && !model.captionSides?.length) {
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

export function drawFamilyBars(ctx: Canvas2D, input: DrawFamilyBarsInput): BarsPaintStats {
  const { model, layout, theme, scrollTop, now, glideDuration, glide, artCache, inspectedKey, selectedKey, focusedKey } = input;
  const tokens = resolveKitTokens(theme);
  ctx.clearRect(0, 0, layout.width, layout.height);

  drawHeader(ctx, model, layout, theme);
  if (model.captionSides?.length && layout.headerHeight > 0) {
    drawCaptionSides(ctx, model.captionSides, layout.width - BARS_PAD_X, layout.headerTop + layout.headerHeight / 2, theme, artCache);
  }
  if (layout.showAxis) drawAxis(ctx, model, layout, theme);

  const { first, last } = visibleBarsRange(layout, scrollTop);
  const truncatedKeys: string[] = [];
  let paintedRows = 0;

  ctx.save();
  ctx.beginPath();
  ctx.rect(layout.bodyLeft, layout.bodyTop, layout.width - layout.bodyLeft, layout.bodyHeight);
  ctx.clip();

  if (model.form === 'net' || (model.form === 'two-sided' && layout.twoSidedLayout !== 'stacked-pair')) {
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
      drawRowSectionLabel(ctx, layout.nameX, y, rowH, item.label, theme);
      continue;
    }

    if (item.kind === 'add-region') {
      const hovered = inspectedKey === item.key;
      if (hovered) drawRowHover(ctx, layout.bodyLeft, y, layout.width - layout.bodyLeft, rowH, theme);
      if (focusedKey === item.key) drawRowFocusRing(ctx, layout.bodyLeft, y, layout.width - layout.bodyLeft, rowH, theme);
      drawAddRow(ctx, layout.nameX, middleY, layout.plotLeft, layout.plotRight, item.label, theme);
      continue;
    }

    const row: BarsDataRow = item;
    const isHovered = inspectedKey === row.key || (row.rightSide && inspectedKey === row.rightSide.key);
    const isSelected = selectedKey === row.key || (row.rightSide && selectedKey === row.rightSide.key);
    const isFocused = focusedKey === row.key || (row.rightSide && focusedKey === row.rightSide.key);

    if (isSelected) drawRowSelected(ctx, layout.bodyLeft, y, layout.width - layout.bodyLeft, rowH, theme);
    else if (isHovered) drawRowHover(ctx, layout.bodyLeft, y, layout.width - layout.bodyLeft, rowH, theme);
    if (isFocused) drawRowFocusRing(ctx, layout.bodyLeft, y, layout.width - layout.bodyLeft, rowH, theme);
    if (model.form === 'net' && i < model.rows.length - 1) {
      drawRowHairline(ctx, layout.bodyLeft + BARS_PAD_X, layout.width - BARS_PAD_X, y + rowH - 1, theme, 'normal');
    }

    drawMediaSlot(ctx, layout.bodyLeft + BARS_PAD_X, middleY, layout.mediaWidth, layout.mediaHeight, row.media, theme, { artCache, radius: 2 });

    if (layout.nameWidth > 0) {
      const regionTools = row.region !== undefined && (isHovered || isFocused);
      const reserveRemove = regionTools ? ROW_REMOVE_SIZE + 4 : 0;
      const nameColor = row.labelMissing || row.labelLoading || row.muted ? theme.muted : row.group === 'auto' ? tokens.autoName : theme.text;
      const fitted = drawNameSlot(
        ctx,
        row.labelMissing ? notRecordedLabel('Name') : row.label,
        layout.nameX,
        y,
        Math.max(12, layout.nameWidth - reserveRemove),
        rowH,
        theme,
        { color: nameColor, weight: model.form === 'net' ? 500 : 400 },
      );
      if (fitted.truncated) truncatedKeys.push(row.key);
      if (regionTools) {
        drawRowGrip(ctx, layout.nameX, middleY, theme);
        drawRowRemoveButton(ctx, layout.nameX + layout.nameWidth, middleY, theme);
      }
    }

    const barY = Math.round(middleY - layout.barHeight / 2);
    const barColor = row.labelMissing ? theme.grey : readableColour(row.color ?? theme.accent, theme.surface);

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
          const captioned = layout.showPair;
          ctx.fillText(ellipsise(formatted.text, layout.readoutWidth, s => textWidth(ctx, s)), rightEdge, captioned ? middleY - 7 : middleY);
          if (captioned) {
            const sub = row.value?.caption ?? ((row.numeric ?? 0) < 0 ? 'Short a day' : (row.numeric ?? 0) > 0 ? 'Surplus a day' : 'Balanced');
            ctx.font = `${LABEL_WEIGHT} 11px ${theme.font}`;
            ctx.fillStyle = theme.muted;
            ctx.fillText(ellipsise(sub, layout.readoutWidth, s => textWidth(ctx, s)), rightEdge, middleY + 8);
          }
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
        drawShareBar(ctx, layout.plotLeft, barY, layout.plotWidth, layout.barHeight, row.segments.map(seg => ({ share: seg.share, color: seg.color, hatched: seg.hatched, label: seg.inlineLabel ?? `${seg.label} ${formatShare(seg.share)}`, shortLabel: formatShare(seg.share) })), theme, 2.5);
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
          layout.rightNameX + layout.rightNameWidth,
          y,
          layout.rightNameWidth,
          rowH,
          theme,
          { align: 'right', color: right.labelMissing || right.labelLoading ? theme.muted : theme.text },
        );
        if (rightName.truncated) truncatedKeys.push(right.key);
        drawMediaSlot(ctx, layout.width - BARS_PAD_X - layout.mediaWidth, middleY, layout.mediaWidth, layout.mediaHeight, right.media, theme, { artCache, radius: 2 });
      }
      continue;
    }

    if (model.form === 'two-sided' && layout.twoSidedLayout === 'stacked-pair') {
      drawStackedPair(ctx, row, layout, theme, barColor, middleY);
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
      if (layout.showReadout && row.readout) {
        ctx.save();
        ctx.font = `500 ${theme.fontSize.body}px ${theme.font}`;
        ctx.fillStyle = theme.text;
        ctx.textAlign = 'right';
        ctx.textBaseline = 'middle';
        ctx.fillText(ellipsise(row.readout.primary, layout.readoutWidth, s => textWidth(ctx, s)), layout.readoutX + layout.readoutWidth, middleY);
        ctx.restore();
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
      ctx.fillStyle = row.readout.tone === 'shortage' ? tokens.shortage : row.readout.tone === 'muted' ? theme.muted : theme.text;
      ctx.fillText(ellipsise(row.readout.primary, Math.max(12, cursorRight - layout.readoutX), s => textWidth(ctx, s)), cursorRight, middleY);
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
        ctx.fillStyle = contrastInkForFill(barColor, theme);
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

  if (layout.legendHeight > 0 && (model.legend?.length || model.scaleUnit)) {
    const entries = (model.legend ?? []).map(entry => ({
      label: entry.share ? `${entry.label} ${entry.share}` : entry.value ? `${entry.label} ${entry.value}` : entry.label,
      color: entry.color,
      ...(entry.hatched ? { hatch: true } : {}),
    }));
    const rowCount = Math.max(1, Math.round(layout.legendHeight / 22));
    if (rowCount > 1 && entries.length > 1) {
      const rowH = layout.legendHeight / rowCount;
      const entryGap = 14;
      ctx.save();
      ctx.font = `500 ${theme.fontSize.small}px ${theme.font}`;
      const rightReserve = model.scaleUnit ? Math.min(Math.floor(layout.width * 0.3), Math.ceil(textWidth(ctx, model.scaleUnit))) + entryGap : 0;
      const row1Limit = Math.max(80, layout.width - BARS_PAD_X * 2 - rightReserve);
      let used = 0;
      let splitAt = entries.length;
      for (let i = 0; i < entries.length; i++) {
        const w = 18 + Math.ceil(textWidth(ctx, entries[i]!.label));
        const next = used + (i > 0 ? entryGap : 0) + w;
        if (i > 0 && next > row1Limit) {
          splitAt = i;
          break;
        }
        used = next;
      }
      ctx.restore();
      drawLegendStrip(ctx, 0, layout.legendTop, layout.width, rowH, {
        entries: entries.slice(0, splitAt),
        ...(model.scaleUnit ? { rightNote: model.scaleUnit } : {}),
        padX: BARS_PAD_X,
        entryGap,
      }, theme);
      if (splitAt < entries.length) {
        drawLegendStrip(ctx, 0, layout.legendTop + rowH, layout.width, rowH, {
          entries: entries.slice(splitAt),
          padX: BARS_PAD_X,
          entryGap,
        }, theme);
      }
    } else {
      drawLegendStrip(ctx, 0, layout.legendTop, layout.width, layout.legendHeight, {
        entries,
        ...(model.scaleUnit ? { rightNote: model.scaleUnit } : {}),
        padX: BARS_PAD_X,
        entryGap: 14,
      }, theme);
    }
  }

  if (model.pickList) drawPickList(ctx, model.pickList, layout, theme, inspectedKey ?? null, focusedKey ?? null);
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
  removeShown: ReadonlySet<string> | null = null,
): BarsHitResult {
  if (point.y < layout.bodyTop || point.y > layout.bodyBottom) return { kind: 'none' };
  const { first, last } = visibleBarsRange(layout, scrollTop);
  for (let i = last - 1; i >= first; i--) {
    const item = model.rows[i]!;
    const baseY = layout.bodyTop + layout.rowTops[i]! - scrollTop;
    const rowH = layout.rowHeights[i]!;
    const y = paintedRowY(item.key, baseY, glide, now, glideDuration);
    if (point.y < y || point.y >= y + rowH) continue;
    if (item.kind === 'section') return { kind: 'none' };
    if (item.kind === 'add-region') return { kind: 'add-region', rowKey: item.key };
    if (item.region !== undefined && removeShown?.has(item.key)) {
      const inside = (box: { readonly x: number; readonly y: number; readonly width: number; readonly height: number }) =>
        point.x >= box.x && point.x <= box.x + box.width && point.y >= box.y && point.y <= box.y + box.height;
      if (inside(rowRemoveBox(layout.nameX + layout.nameWidth, y + rowH / 2))) return { kind: 'remove-region', rowKey: item.key };
      if (inside(rowGripBox(layout.nameX, y + rowH / 2))) return { kind: 'grip-region', rowKey: item.key };
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
  const items: ArtUrlInput[] = [];
  for (let i = first; i < last; i++) {
    const item = model.rows[i]!;
    if (item.kind !== 'data') continue;
    items.push(item.media, item.rightSide?.media);
  }
  for (const side of model.captionSides ?? []) items.push(side.media);
  return collectArtUrls(items);
}
