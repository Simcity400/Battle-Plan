import { ellipsise } from '../../charts/labels.ts';
import { textWidth, type Canvas2D } from '../../charts/textWidth.ts';
import {
  drawBar,
  drawChevron,
  drawFlagSlot,
  drawMissingMark,
  drawPartialTab,
  drawRowFocusRing,
  drawRowHairline,
  drawRowHover,
  drawRowSelected,
  drawScrollFade,
  resolveKitTokens,
  type FlagSlotState,
} from '../../draw/kit.ts';
import { toFlagSlotState } from '../art/adapters.ts';
import type { LaneArtCache } from '../art/laneCache.ts';
import type { FamilyTheme } from '../contract.ts';
import { drawHatchLines } from '../valueCanvas.ts';
import type { ArtResult } from '../plan.ts';
import {
  firstRowFrom,
  type BreakdownLayout,
  type MosaicTile,
  type MosaicViewLayout,
  type PartsViewLayout,
  type Rect,
  type RingViewLayout,
  type TotalLineLayout,
} from './layout.ts';
import type { BreakdownComparison } from './model.ts';

export interface BreakdownPaintInput {
  readonly layout: BreakdownLayout;
  readonly theme: FamilyTheme;
  readonly hour: number;
  readonly scrollTop: number;
  readonly inspectedKey: string | null;
  readonly selectedKey: string | null;
  readonly focusedKey: string | null;
  readonly artCache?: LaneArtCache;
}

function fitText(ctx: Canvas2D, text: string, maxWidth: number): string {
  if (maxWidth <= 0) return '';
  return ellipsise(text, maxWidth, s => textWidth(ctx, s));
}

function flagStateOf(
  tag: string | undefined,
  flagArt: ArtResult | undefined,
  _hour: number,
  _theme: FamilyTheme,
  _artCache: LaneArtCache | undefined,
): FlagSlotState | null {
  if (flagArt) {
    return toFlagSlotState(flagArt, tag);
  }
  if (!tag) return null;
  return { kind: 'pending' };
}

function drawComparison(ctx: Canvas2D, comparison: BreakdownComparison, box: Rect, cy: number, theme: FamilyTheme): void {
  const right = box.x + box.width;
  ctx.save();
  ctx.textBaseline = 'middle';
  ctx.textAlign = 'right';
  ctx.font = `500 12px ${theme.font}`;
  ctx.fillStyle = theme.text;
  const valW = textWidth(ctx, comparison.value);
  ctx.fillText(comparison.value, right, cy);
  ctx.font = `400 12px ${theme.font}`;
  ctx.fillStyle = theme.muted;
  ctx.fillText(comparison.label, right - valW - 6, cy);
  ctx.restore();
}

function drawTotalLine(ctx: Canvas2D, line: TotalLineLayout, theme: FamilyTheme): void {
  const { rect, total, periodLabel, comparison, comparisonRect, partialTabRect } = line;
  const cy = rect.y + rect.height / 2;
  ctx.save();
  ctx.textBaseline = 'middle';
  ctx.textAlign = 'left';

  let cursor = rect.x;

  ctx.font = `600 22px ${theme.font}`;
  ctx.fillStyle = theme.text;
  ctx.fillText(total.text, cursor, cy);
  cursor += textWidth(ctx, total.text);

  if (total.unit) {
    cursor += 4;
    ctx.font = `400 12px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.fillText(total.unit, cursor, cy + 3);
    cursor += textWidth(ctx, total.unit);
  }

  if (partialTabRect) {
    drawPartialTab(ctx, partialTabRect.x, cy + 1, theme);
    cursor = partialTabRect.x + partialTabRect.width;
  }

  let rightEdge = rect.x + rect.width;
  if (comparison && comparisonRect) {
    drawComparison(ctx, comparison, comparisonRect, cy + 2, theme);
    rightEdge = comparisonRect.x - 14;
  }

  if (periodLabel && rightEdge > cursor + 24) {
    ctx.textAlign = 'left';
    ctx.font = `400 12px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    const shown = fitText(ctx, periodLabel, rightEdge - cursor - 12);
    ctx.fillText(shown, cursor + 12, cy + 2);
  }

  ctx.restore();
}

function drawPartsView(ctx: Canvas2D, input: BreakdownPaintInput, layout: PartsViewLayout): void {
  const { theme, hour, scrollTop, inspectedKey, selectedKey, focusedKey, artCache } = input;
  const tokens = resolveKitTokens(theme);
  drawTotalLine(ctx, layout.totalLine, theme);

  const viewTop = layout.bodyTop;
  const viewH = Math.max(0, layout.height - viewTop);
  ctx.save();
  ctx.beginPath();
  ctx.rect(0, viewTop, layout.width, viewH);
  ctx.clip();

  const trackFill = tokens.partsTrack;
  const treeStroke = tokens.hairlineStrong;

  for (let at = firstRowFrom(layout.rows, scrollTop - 2); at < layout.rows.length; at++) {
    const row = layout.rows[at]!;
    const sy = viewTop + row.y - scrollTop;
    if (sy > layout.height + 2) break;
    const cy = sy + row.height / 2;

    const inspectedSegment = inspectedKey !== null && row.segments.length > 0 && row.segments.some(s => s.key === inspectedKey);
    const isSelected = selectedKey !== null && selectedKey === row.key;
    const isInspected = inspectedKey !== null && (inspectedKey === row.key || inspectedSegment);
    const isFocused = focusedKey !== null && focusedKey === row.key;

    if (isSelected) {
      drawRowSelected(ctx, layout.padX - 4, sy, layout.width - layout.padX * 2 + 8, row.height, theme, 0);
    } else if (isInspected) {
      drawRowHover(ctx, layout.padX - 4, sy, layout.width - layout.padX * 2 + 8, row.height, theme, 0);
    }

    if (row.depth === 0) {
      if (row.chevronRect) {
        drawChevron(ctx, row.chevronRect.x - 1, cy, row.expanded, theme);
      }
      let cursorX = row.nameX;
      const flagState = flagStateOf(row.part.flag, row.part.flagArt, hour, theme, artCache);
      if (flagState && !layout.narrow) {
        drawFlagSlot(ctx, cursorX, Math.round(cy - 6), flagState, theme, { width: 18, height: 12 });
        cursorX += 24;
      } else {
        ctx.save();
        ctx.fillStyle = row.swatchColor;
        ctx.beginPath();
        ctx.roundRect(cursorX, Math.round(cy - 5), 10, 10, 2);
        ctx.fill();
        ctx.restore();
        cursorX += 18;
      }

      const textMaxW = Math.max(10, row.nameX + row.nameWidth - cursorX);
      ctx.save();
      ctx.font = `400 12px ${theme.font}`;
      ctx.fillStyle = theme.text;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillText(fitText(ctx, row.part.name, textMaxW), cursorX, cy);
      ctx.restore();

      const barH = 12;
      const barY = Math.round(cy - barH / 2);
      if (row.part.notRecorded || row.part.value === null) {
        drawMissingMark(ctx, row.barX, cy, row.barWidth, theme);
      } else {
        ctx.save();
        ctx.fillStyle = trackFill;
        ctx.beginPath();
        ctx.roundRect(row.barX, barY, row.barWidth, barH, [0, 3, 3, 0]);
        ctx.fill();
        ctx.restore();

        if (row.segments.length > 0) {
          for (let i = 0; i < row.segments.length; i++) {
            const seg = row.segments[i]!;
            const isLast = i === row.segments.length - 1;
            const segActive = inspectedKey === seg.key;
            const segAlpha = inspectedSegment && !segActive ? 0.65 : 1;
            if (isLast) {
              drawBar(ctx, seg.x, barY, seg.width, barH, seg.color, 'right', segAlpha);
            } else {
              ctx.save();
              ctx.globalAlpha = segAlpha;
              ctx.fillStyle = seg.color;
              ctx.fillRect(seg.x, barY, seg.width, barH);
              ctx.restore();
            }
            if (seg.inlineLabel) {
              ctx.save();
              ctx.font = `500 10.5px ${theme.font}`;
              ctx.fillStyle = seg.ink;
              ctx.textAlign = 'left';
              ctx.textBaseline = 'middle';
              ctx.fillText(seg.inlineLabel, seg.x + 5, cy);
              ctx.restore();
            }
          }
        } else if (row.barFillWidth > 0) {
          drawBar(ctx, row.barX, barY, row.barFillWidth, barH, row.swatchColor, 'right');
        }
      }

      if (!row.part.notRecorded) {
        ctx.save();
        ctx.font = `500 12px ${theme.font}`;
        ctx.fillStyle = theme.text;
        ctx.textAlign = 'right';
        ctx.textBaseline = 'middle';
        ctx.fillText(fitText(ctx, row.part.text, row.valueWidth), row.valueX + row.valueWidth, cy);
        if (row.shareWidth > 0 && row.shareText) {
          ctx.font = `400 11px ${theme.font}`;
          ctx.fillStyle = theme.muted;
          ctx.fillText(row.shareText, row.shareX + row.shareWidth, cy);
        }
        ctx.restore();
      }
    } else {
      const treeX = layout.padX + (row.depth === 1 ? 6.5 : 26.5);
      ctx.save();
      ctx.strokeStyle = treeStroke;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(treeX, sy);
      ctx.lineTo(treeX, row.isLastChild ? cy : sy + row.height);
      ctx.moveTo(treeX, cy);
      ctx.lineTo(treeX + 8, cy);
      ctx.stroke();
      ctx.restore();

      ctx.save();
      ctx.font = row.depth === 1 ? `400 12px ${theme.font}` : `400 11.5px ${theme.font}`;
      ctx.fillStyle = row.depth === 1 ? theme.text : theme.muted;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillText(fitText(ctx, row.part.name, row.nameWidth), row.nameX, cy);
      ctx.restore();

      const barH = row.depth === 1 ? 8 : 6;
      const barY = Math.round(cy - barH / 2);
      if (row.part.notRecorded || row.part.value === null) {
        drawMissingMark(ctx, row.barX, cy, row.barWidth, theme);
      } else {
        ctx.save();
        ctx.fillStyle = trackFill;
        ctx.beginPath();
        ctx.roundRect(row.barX, barY, row.barWidth, barH, [0, 2.5, 2.5, 0]);
        ctx.fill();
        ctx.restore();
        if (row.barFillWidth > 0) {
          drawBar(ctx, row.barX, barY, row.barFillWidth, barH, row.swatchColor, 'right', row.depth === 1 ? 0.62 : 0.45);
        }
      }

      if (!row.part.notRecorded) {
        ctx.save();
        ctx.font = `400 12px ${theme.font}`;
        ctx.fillStyle = theme.text;
        ctx.textAlign = 'right';
        ctx.textBaseline = 'middle';
        ctx.fillText(fitText(ctx, row.part.text, row.valueWidth), row.valueX + row.valueWidth, cy);
        if (row.shareWidth > 0 && row.shareText) {
          ctx.font = `400 11px ${theme.font}`;
          ctx.fillStyle = theme.muted;
          ctx.fillText(row.shareText, row.shareX + row.shareWidth, cy);
        }
        ctx.restore();
      }
    }

    if (isFocused) {
      drawRowFocusRing(ctx, layout.padX - 4, sy, layout.width - layout.padX * 2 + 8, row.height, theme, 0);
    }
  }

  ctx.restore();

  if (scrollTop + layout.height < layout.contentHeight - 2) {
    drawScrollFade(ctx, 0, layout.height - 22, layout.width, 22, theme, 'bottom');
  }
}

function drawArcText(
  ctx: Canvas2D,
  text: string,
  cx: number,
  cy: number,
  r: number,
  midAngle: number,
  reversed: boolean,
  ink: string,
  font: string,
): void {
  ctx.save();
  ctx.font = font;
  ctx.fillStyle = ink;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  const chars = [...text];
  const widths = chars.map(ch => textWidth(ctx, ch));
  const totalW = widths.reduce((a, b) => a + b, 0);
  const totalAngle = totalW / Math.max(1, r);
  let cur = reversed ? midAngle + totalAngle / 2 : midAngle - totalAngle / 2;
  for (let i = 0; i < chars.length; i++) {
    const halfA = (widths[i]! / 2) / Math.max(1, r);
    const charA = reversed ? cur - halfA : cur + halfA;
    ctx.save();
    ctx.translate(cx + r * Math.cos(charA), cy + r * Math.sin(charA));
    ctx.rotate(charA + (reversed ? -Math.PI / 2 : Math.PI / 2));
    ctx.fillText(chars[i]!, 0, 0);
    ctx.restore();
    cur = reversed ? cur - halfA * 2 : cur + halfA * 2;
  }
  ctx.restore();
}

function wedgePath(ctx: Canvas2D, cx: number, cy: number, rIn: number, rOut: number, a0: number, a1: number): void {
  ctx.beginPath();
  ctx.arc(cx, cy, rOut, a0, a1);
  ctx.arc(cx, cy, rIn, a1, a0, true);
  ctx.closePath();
}

function drawRingView(ctx: Canvas2D, input: BreakdownPaintInput, layout: RingViewLayout): void {
  const { theme, scrollTop, inspectedKey, selectedKey, focusedKey } = input;

  if (layout.topLine) {
    const { rect, text, comparison, comparisonRect } = layout.topLine;
    const cy = rect.y + rect.height / 2;
    ctx.save();
    ctx.textBaseline = 'middle';
    let rightEdge = rect.x + rect.width;
    if (comparison && comparisonRect) {
      drawComparison(ctx, comparison, comparisonRect, cy, theme);
      rightEdge = comparisonRect.x - 12;
    }
    if (text) {
      ctx.textAlign = 'left';
      ctx.font = `400 12px ${theme.font}`;
      ctx.fillStyle = theme.muted;
      ctx.fillText(fitText(ctx, text, Math.max(20, rightEdge - rect.x)), rect.x, cy);
    }
    ctx.restore();
    drawRowHairline(ctx, rect.x, rect.x + rect.width, rect.y + rect.height, theme, 'normal');
  }

  const activeKey = inspectedKey ?? focusedKey;
  const activeParent = activeKey
    ? layout.outerSlices.find(s => s.key === activeKey)?.parentKey ?? activeKey
    : null;

  for (const slice of layout.innerSlices) {
    const dimmed = activeParent !== null && slice.key !== activeParent && slice.key !== activeKey;
    ctx.save();
    wedgePath(ctx, layout.cx, layout.cy, layout.r1i, layout.r1o, slice.a0, slice.a1);
    ctx.globalAlpha = dimmed ? 0.45 : 1;
    ctx.fillStyle = slice.color;
    ctx.fill();
    if (slice.hatched) {
      ctx.clip();
      drawHatchLines(ctx, layout.cx - layout.r1o, layout.cy - layout.r1o, layout.r1o * 2, layout.r1o * 2, theme);
    }
    ctx.restore();

    if (slice.label) {
      ctx.save();
      ctx.globalAlpha = dimmed ? 0.5 : 1;
      ctx.fillStyle = slice.label.ink;
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.font = `600 ${slice.label.fontSize}px ${theme.font}`;
      if (slice.label.line2) {
        ctx.fillText(slice.label.line1, slice.label.x, slice.label.y - 6.5);
        ctx.font = `400 11px ${theme.font}`;
        ctx.fillText(slice.label.line2, slice.label.x, slice.label.y + 7);
      } else {
        ctx.fillText(slice.label.line1, slice.label.x, slice.label.y);
      }
      ctx.restore();
    }
  }

  const outerFont = `600 10.5px ${theme.font}`;
  const outerMidR = (layout.r2i + layout.r2o) / 2;
  for (const slice of layout.outerSlices) {
    const dimmed = activeParent !== null && slice.key !== activeKey && slice.parentKey !== activeParent;
    const isExact = activeKey === slice.key;
    ctx.save();
    wedgePath(ctx, layout.cx, layout.cy, layout.r2i, layout.r2o, slice.a0, slice.a1);
    ctx.globalAlpha = dimmed ? 0.35 : isExact ? 1 : 0.92;
    ctx.fillStyle = slice.color;
    ctx.fill();
    if (slice.hatched) {
      ctx.clip();
      drawHatchLines(ctx, layout.cx - layout.r2o, layout.cy - layout.r2o, layout.r2o * 2, layout.r2o * 2, theme);
    }
    ctx.restore();

    if (slice.arcLabel) {
      ctx.save();
      ctx.globalAlpha = dimmed ? 0.4 : 1;
      drawArcText(
        ctx,
        slice.arcLabel.text,
        layout.cx,
        layout.cy,
        outerMidR,
        slice.arcLabel.midAngle,
        slice.arcLabel.reversed,
        slice.arcLabel.ink,
        outerFont,
      );
      ctx.restore();
    }
  }

  ctx.save();
  ctx.fillStyle = theme.surface;
  ctx.beginPath();
  ctx.arc(layout.cx, layout.cy, Math.max(1, layout.r1i - 0.5), 0, Math.PI * 2);
  ctx.fill();

  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  const maxHoleW = Math.max(20, layout.r1i * 1.75);
  if (layout.center.verb) {
    ctx.font = `400 11px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.fillText(fitText(ctx, layout.center.verb, maxHoleW), layout.cx, layout.cy - 16);
  }
  ctx.font = `600 ${layout.center.valueFontSize}px ${theme.font}`;
  ctx.fillStyle = theme.text;
  const valY = layout.center.verb ? layout.cy + 4 : layout.cy;
  ctx.fillText(fitText(ctx, layout.center.value, maxHoleW), layout.cx, valY);
  if (layout.center.partialTabRect) drawPartialTab(ctx, layout.center.partialTabRect.x, valY, theme);
  if (layout.center.unitLine) {
    ctx.font = `400 11px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.fillText(fitText(ctx, layout.center.unitLine, maxHoleW), layout.cx, layout.cy + 21);
  }
  ctx.restore();

  if (layout.sideList) {
    const list = layout.sideList;
    ctx.save();
    ctx.beginPath();
    ctx.rect(list.x - 4, list.top, list.width + 8, list.height);
    ctx.clip();

    const valW = 70;
    const shareW = 44;
    const valRight = list.x + list.width - shareW - 6;
    const shareRight = list.x + list.width;

    for (let at = firstRowFrom(list.rows, scrollTop - 2); at < list.rows.length; at++) {
      const row = list.rows[at]!;
      const sy = list.top + row.y - scrollTop;
      if (sy > list.top + list.height + 2) break;
      const cy = sy + row.height / 2;

      const isSelected = selectedKey === row.key;
      const isInspected = inspectedKey === row.key;
      const isFocused = focusedKey === row.key;

      if (isSelected) {
        drawRowSelected(ctx, list.x - 4, sy, list.width + 8, row.height, theme, 0);
      } else if (isInspected) {
        drawRowHover(ctx, list.x - 4, sy, list.width + 8, row.height, theme, 0);
      }

      if (row.depth === 0) {
        if (row.chevronRect) {
          drawChevron(ctx, row.chevronRect.x - 1, cy, row.expanded, theme);
        }
        const swatchX = list.x + 14;
        if (row.part.notRecorded) {
          drawMissingMark(ctx, swatchX, cy, 10, theme);
        } else {
          ctx.save();
          ctx.fillStyle = row.swatchColor;
          ctx.beginPath();
          ctx.roundRect(swatchX, Math.round(cy - 5), 10, 10, 2);
          ctx.fill();
          ctx.restore();
        }
        const nameX = swatchX + 16;
        const nameW = Math.max(20, valRight - valW - 8 - nameX);
        ctx.save();
        ctx.font = `400 12px ${theme.font}`;
        ctx.fillStyle = row.part.notRecorded ? theme.muted : theme.text;
        ctx.textAlign = 'left';
        ctx.textBaseline = 'middle';
        ctx.fillText(fitText(ctx, row.part.name, nameW), nameX, cy);

        ctx.textAlign = 'right';
        if (row.part.notRecorded) {
          ctx.font = `400 11px ${theme.font}`;
          ctx.fillStyle = theme.muted;
          ctx.fillText('Not recorded', shareRight, cy);
        } else {
          ctx.font = `500 12px ${theme.font}`;
          ctx.fillStyle = theme.text;
          ctx.fillText(fitText(ctx, row.part.text, valW), valRight, cy);
          if (row.shareText) {
            ctx.font = `400 11px ${theme.font}`;
            ctx.fillStyle = theme.muted;
            ctx.fillText(row.shareText, shareRight, cy);
          }
        }
        ctx.restore();
      } else {
        const swatchX = list.x + (row.depth === 1 ? 30 : 44);
        ctx.save();
        ctx.fillStyle = row.swatchColor;
        ctx.beginPath();
        ctx.roundRect(swatchX, Math.round(cy - 4), 8, 8, 2);
        ctx.fill();
        ctx.restore();

        const nameX = swatchX + 14;
        const nameW = Math.max(20, valRight - valW - 8 - nameX);
        ctx.save();
        ctx.font = `400 11.5px ${theme.font}`;
        ctx.fillStyle = theme.muted;
        ctx.textAlign = 'left';
        ctx.textBaseline = 'middle';
        ctx.fillText(fitText(ctx, row.part.name, nameW), nameX, cy);

        ctx.textAlign = 'right';
        if (row.part.notRecorded) {
          ctx.font = `400 10.5px ${theme.font}`;
          ctx.fillText('Not recorded', shareRight, cy);
        } else {
          ctx.fillText(fitText(ctx, row.part.text, valW), valRight, cy);
          if (row.shareText) {
            ctx.font = `400 10.5px ${theme.font}`;
            ctx.fillText(row.shareText, shareRight, cy);
          }
        }
        ctx.restore();
      }

      if (isFocused) {
        drawRowFocusRing(ctx, list.x - 4, sy, list.width + 8, row.height, theme, 0);
      }
    }
    ctx.restore();

    if (scrollTop + list.height < list.contentHeight - 2) {
      drawScrollFade(ctx, list.x - 4, list.top + list.height - 22, list.width + 8, 22, theme, 'bottom');
    }
  }
}

function drawMosaicTile(
  ctx: Canvas2D,
  tile: MosaicTile,
  theme: FamilyTheme,
  inspectedKey: string | null,
  focusedKey: string | null,
  radius: number,
): void {
  const active = inspectedKey === tile.key || focusedKey === tile.key;
  ctx.save();
  ctx.beginPath();
  ctx.roundRect(tile.x, tile.y, tile.width, tile.height, radius);
  ctx.fillStyle = tile.color;
  ctx.fill();
  if (tile.hatched && !tile.isGroup) {
    ctx.save();
    ctx.clip();
    drawHatchLines(ctx, tile.x, tile.y, tile.width, tile.height, theme);
    ctx.restore();
  }
  if (active && !tile.isGroup) {
    ctx.strokeStyle = theme.text;
    ctx.lineWidth = 1.5;
    ctx.stroke();
  }
  ctx.restore();

  if (tile.isGroup) {
    const headerCy = tile.y + tile.headerHeight / 2;
    ctx.save();
    ctx.textBaseline = 'middle';
    let rightLimit = tile.x + tile.width - 6;
    if (tile.headerRight) {
      ctx.font = `400 10.5px ${theme.font}`;
      ctx.fillStyle = tile.ink;
      ctx.globalAlpha = 0.85;
      ctx.textAlign = 'right';
      const rw = textWidth(ctx, tile.headerRight);
      ctx.fillText(tile.headerRight, rightLimit, headerCy);
      rightLimit -= rw + 8;
    }
    ctx.globalAlpha = 1;
    ctx.font = `600 11.5px ${theme.font}`;
    ctx.fillStyle = tile.ink;
    ctx.textAlign = 'left';
    ctx.fillText(fitText(ctx, tile.part.name, Math.max(12, rightLimit - (tile.x + 6))), tile.x + 6, headerCy);
    ctx.restore();

    for (const child of tile.children) {
      drawMosaicTile(ctx, child, theme, inspectedKey, focusedKey, 2);
    }
    return;
  }

  if (tile.labelMode === 'none') return;
  const pad = 6;
  const maxW = Math.max(8, tile.width - pad * 2);
  ctx.save();
  ctx.fillStyle = tile.ink;
  ctx.textBaseline = 'middle';
  if (tile.labelMode === 'full') {
    ctx.textAlign = 'left';
    ctx.font = `600 11.5px ${theme.font}`;
    ctx.fillText(fitText(ctx, tile.part.name, maxW), tile.x + pad, tile.y + 13);
    ctx.globalAlpha = 0.85;
    ctx.font = `400 10.5px ${theme.font}`;
    ctx.fillText(fitText(ctx, tile.valueLine, maxW), tile.x + pad, tile.y + 28);
  } else {
    ctx.textAlign = 'center';
    ctx.font = `600 11px ${theme.font}`;
    ctx.fillText(fitText(ctx, tile.part.name, maxW), tile.x + tile.width / 2, tile.y + tile.height / 2);
  }
  ctx.restore();
}

function drawMosaicView(ctx: Canvas2D, input: BreakdownPaintInput, layout: MosaicViewLayout): void {
  const { theme, inspectedKey, focusedKey } = input;
  drawTotalLine(ctx, layout.totalLine, theme);

  for (const tile of layout.tiles) {
    drawMosaicTile(ctx, tile, theme, inspectedKey, focusedKey, 3);
  }
}

export function drawBreakdown(ctx: Canvas2D, input: BreakdownPaintInput): void {
  const { layout, theme } = input;
  ctx.save();
  ctx.fillStyle = theme.surface;
  ctx.fillRect(0, 0, layout.width, layout.height);
  if (layout.view === 'parts') {
    drawPartsView(ctx, input, layout);
  } else if (layout.view === 'ring') {
    drawRingView(ctx, input, layout);
  } else {
    drawMosaicView(ctx, input, layout);
  }
  ctx.restore();
}
