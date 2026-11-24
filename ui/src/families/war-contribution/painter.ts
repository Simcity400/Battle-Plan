import { ellipsise } from '../../charts/labels.ts';
import { textWidth, type Canvas2D } from '../../charts/textWidth.ts';
import {
  drawBar,
  drawFlagSlot,
  drawPartialTab,
  drawRowFocusRing,
  drawRowHairline,
  drawRowHover,
  drawRowSelected,
  drawScrollFade,
  drawTrackBar,
  resolveKitTokens,
} from '../../draw/kit.ts';
import { toFlagSlotState } from '../art/adapters.ts';
import type { FamilyTheme } from '../contract.ts';
import type { ArtResult } from '../plan.ts';
import type {
  WCMeasureLayout,
  WCSidesLayout,
  WCSidesPanelLayout,
  WarContributionLayout,
} from './layout.ts';
import type { WarContributionModel } from './model.ts';

export interface WarContributionPaintInput {
  readonly model: WarContributionModel;
  readonly layout: WarContributionLayout;
  readonly theme: FamilyTheme;
  readonly scrollTop: number;
  readonly scrollLeftLeft: number;
  readonly scrollLeftRight: number;
  readonly inspectedKey: string | null;
  readonly focusedKey: string | null;
}

function fit(ctx: Canvas2D, text: string, maxW: number): string {
  if (maxW <= 0) return '';
  return ellipsise(text, maxW, s => textWidth(ctx, s));
}

function drawFlag(
  ctx: Canvas2D,
  x: number,
  y: number,
  art: ArtResult | undefined,
  tag: string | null,
  theme: FamilyTheme,
  width = 20,
  height = 13,
): void {
  const state = art ? toFlagSlotState(art, tag ?? undefined) : (tag ? { kind: 'missing' as const, tag } : { kind: 'pending' as const });
  drawFlagSlot(ctx, x, y, state, theme, { width, height });
}

function drawSidePanel(
  ctx: Canvas2D,
  panel: WCSidesPanelLayout,
  model: WarContributionModel,
  theme: FamilyTheme,
  scrollTop: number,
  scrollLeft: number,
  inspectedKey: string | null,
  focusedKey: string | null,
): void {
  const tokens = resolveKitTokens(theme);
  const maxScroll = Math.max(0, panel.measureTotalWidth - panel.measureViewportWidth);
  const sl = Math.max(0, Math.min(maxScroll, scrollLeft));
  const quietZero = tokens.dark ? 'rgba(245, 245, 245, 0.25)' : 'rgba(15, 23, 42, 0.28)';

  const headCy = panel.headerY + 17;
  drawFlag(ctx, panel.x, Math.round(headCy - 6.5), panel.summary.leadFlagArt, panel.summary.leadTag, theme, 20, 13);
  ctx.save();
  ctx.font = `600 13px ${theme.font}`;
  ctx.fillStyle = theme.text;
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  ctx.fillText(fit(ctx, panel.summary.leadName, Math.max(20, panel.countryColWidth - 32)), panel.x + 26, headCy);
  ctx.restore();

  ctx.save();
  ctx.beginPath();
  ctx.rect(panel.measureViewportX, panel.headerY, panel.measureViewportWidth, panel.height);
  ctx.clip();

  for (const col of panel.columns) {
    const colRight = panel.measureViewportX + col.x - sl + col.width - 8;
    if (colRight < panel.measureViewportX - 20 || colRight - col.width > panel.measureViewportX + panel.measureViewportWidth + 20) continue;
    ctx.save();
    ctx.font = `${col.sorted ? 600 : 500} 12px ${theme.font}`;
    ctx.fillStyle = col.sorted ? theme.text : theme.muted;
    ctx.textAlign = 'right';
    ctx.textBaseline = 'middle';
    const label = col.sorted ? `${col.label} ↓` : col.label;
    ctx.fillText(fit(ctx, label, col.width - 10), colRight, headCy);
    ctx.restore();
  }
  ctx.restore();

  drawRowHairline(ctx, panel.x, panel.x + panel.width, panel.warShareY, theme, 'strong');

  const warCy = panel.warShareY + 13;
  ctx.save();
  ctx.fillStyle = tokens.headerBand;
  ctx.fillRect(panel.x, panel.warShareY, panel.width, 26);
  ctx.font = `600 11.5px ${theme.font}`;
  ctx.fillStyle = theme.muted;
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  ctx.fillText('Share of the war', panel.x + 2, warCy);
  ctx.restore();

  ctx.save();
  ctx.beginPath();
  ctx.rect(panel.measureViewportX, panel.warShareY, panel.measureViewportWidth, 26);
  ctx.clip();
  for (const col of panel.columns) {
    const colRight = panel.measureViewportX + col.x - sl + col.width - 8;
    if (colRight < panel.measureViewportX - 20 || colRight - col.width > panel.measureViewportX + panel.measureViewportWidth + 20) continue;
    const sum = panel.summary.totals[col.key];
    const txt = model.show === 'totals' ? sum.totalText : sum.warShareText;
    ctx.save();
    ctx.font = `600 12px ${theme.font}`;
    ctx.fillStyle = txt === '—' ? quietZero : theme.text;
    ctx.textAlign = 'right';
    ctx.textBaseline = 'middle';
    const textRight = sum.hasPartial ? colRight - 10 : colRight;
    ctx.fillText(txt, textRight, warCy);
    if (sum.hasPartial) {
      drawPartialTab(ctx, colRight - 6, warCy, theme);
    }
    ctx.restore();
  }
  ctx.restore();

  drawRowHairline(ctx, panel.x, panel.x + panel.width, panel.subHeaderY, theme, 'normal');

  const subCy = panel.subHeaderY + 10;
  ctx.save();
  ctx.font = `500 11px ${theme.font}`;
  ctx.fillStyle = theme.muted;
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  ctx.fillText(model.show === 'totals' ? 'Amounts' : 'Share of the side', panel.x + 2, subCy);
  ctx.restore();

  const rowsClipH = Math.max(0, panel.y + panel.height - panel.rowsTop - 6);
  ctx.save();
  ctx.beginPath();
  ctx.rect(panel.x - 2, panel.rowsTop, panel.width + 4, rowsClipH);
  ctx.clip();

  for (const row of panel.rows) {
    const sy = panel.rowsTop + row.y - scrollTop;
    if (sy + row.height < panel.rowsTop - 2 || sy > panel.rowsTop + rowsClipH + 2) continue;
    const cy = sy + row.height / 2;

    if (inspectedKey === row.member.key) {
      drawRowHover(ctx, panel.x, sy, panel.width, row.height, theme, 0);
    }

    drawFlag(ctx, panel.x, Math.round(cy - 6.5), row.member.flagArt, row.member.tag, theme, 20, 13);
    ctx.save();
    ctx.font = `400 12px ${theme.font}`;
    ctx.fillStyle = theme.text;
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    ctx.fillText(fit(ctx, row.member.name, Math.max(20, panel.countryColWidth - 32)), panel.x + 26, cy);
    ctx.restore();

    ctx.save();
    ctx.beginPath();
    ctx.rect(panel.measureViewportX, sy, panel.measureViewportWidth, row.height);
    ctx.clip();
    for (const col of panel.columns) {
      const colLeft = panel.measureViewportX + col.x - sl;
      const colRight = colLeft + col.width - 8;
      if (colRight < panel.measureViewportX - 20 || colLeft > panel.measureViewportX + panel.measureViewportWidth + 20) continue;
      const cell = row.member.cells[col.key];

      if (col.sorted) {
        const barW = 52;
        const barX = colLeft + 4;
        const frac = Math.max(0, Math.min(1, cell.shareOfSide ?? 0));
        if (!cell.isZero) {
          drawTrackBar(ctx, barX, cy, barW, 4, frac, row.member.color, theme);
        }
      }

      ctx.save();
      ctx.font = `${col.sorted ? 500 : 400} 12px ${theme.font}`;
      ctx.fillStyle = cell.isZero ? quietZero : theme.text;
      ctx.textAlign = 'right';
      ctx.textBaseline = 'middle';
      const valRight = cell.hasPartial ? colRight - 10 : colRight;
      ctx.fillText(cell.displayText, valRight, cy);
      if (cell.hasPartial) {
        drawPartialTab(ctx, colRight - 6, cy, theme);
      }
      ctx.restore();
    }
    ctx.restore();

    drawRowHairline(ctx, panel.x, panel.x + panel.width, sy + row.height, theme, 'normal');
    if (focusedKey === row.member.key) {
      drawRowFocusRing(ctx, panel.x, sy, panel.width, row.height, theme, 0);
    }
  }
  ctx.restore();

  if (sl < maxScroll - 1) {
    drawScrollFade(ctx, panel.measureViewportX + panel.measureViewportWidth - 40, panel.headerY, 40, panel.height - 6, theme, 'right');
  }
  if (maxScroll > 0) {
    const trackX = panel.measureViewportX;
    const trackY = panel.y + panel.height - 4;
    const trackW = panel.measureViewportWidth;
    const thumbW = Math.max(24, (panel.measureViewportWidth / panel.measureTotalWidth) * trackW);
    const thumbX = trackX + (sl / maxScroll) * (trackW - thumbW);
    ctx.save();
    ctx.fillStyle = tokens.barTrack;
    ctx.beginPath();
    ctx.roundRect(trackX, trackY, trackW, 3, 1.5);
    ctx.fill();
    ctx.fillStyle = tokens.playhead;
    ctx.beginPath();
    ctx.roundRect(thumbX, trackY, thumbW, 3, 1.5);
    ctx.fill();
    ctx.restore();
  }
}

function drawSidesView(ctx: Canvas2D, input: WarContributionPaintInput, layout: WCSidesLayout): void {
  const { model, theme, scrollTop, scrollLeftLeft, scrollLeftRight, inspectedKey, focusedKey } = input;
  const tokens = resolveKitTokens(theme);
  drawSidePanel(ctx, layout.panels[0], model, theme, scrollTop, scrollLeftLeft, inspectedKey, focusedKey);
  drawSidePanel(ctx, layout.panels[1], model, theme, scrollTop, scrollLeftRight, inspectedKey, focusedKey);

  if (layout.dividerX !== null) {
    const px = Math.floor(layout.dividerX) + 0.5;
    ctx.save();
    ctx.strokeStyle = tokens.hairlineStrong;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(px, 6);
    ctx.lineTo(px, layout.height - 10);
    ctx.stroke();
    ctx.restore();
  }
}

function drawMeasureView(ctx: Canvas2D, input: WarContributionPaintInput, layout: WCMeasureLayout): void {
  const { model, theme, scrollTop, inspectedKey, focusedKey } = input;
  const tokens = resolveKitTokens(theme);

  if (layout.listArea) {
    const list = layout.listArea;
    for (const row of layout.listRows) {
      const cy = row.y + row.height / 2;
      if (row.dividerAbove) {
        drawRowHairline(ctx, list.x, list.x + list.width, row.y - 4, theme, 'normal');
      }
      const isSelected = row.key === layout.selectedMeasure;
      const isHovered = inspectedKey === `measure:${row.key}`;
      if (isSelected) {
        drawRowSelected(ctx, list.x, row.y, list.width, row.height, theme, 0);
      } else if (isHovered) {
        drawRowHover(ctx, list.x, row.y, list.width, row.height, theme, 0);
      }

      ctx.save();
      ctx.font = `${isSelected ? 600 : 400} 12px ${theme.font}`;
      ctx.fillStyle = isSelected ? theme.text : theme.muted;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      const labelMax = row.comparable ? Math.max(20, list.width - 108) : list.width - 16;
      ctx.fillText(fit(ctx, row.label, labelMax), list.x + 8, cy);
      ctx.restore();

      if (row.comparable) {
        const barW = 92;
        const barH = 6;
        const barX = list.x + list.width - barW - 8;
        const barY = Math.round(cy - barH / 2);
        const total = row.leftFraction + row.rightFraction;
        ctx.save();
        ctx.fillStyle = tokens.barTrack;
        ctx.beginPath();
        ctx.roundRect(barX, barY, barW, barH, 3);
        ctx.fill();
        if (total > 0) {
          const leftW = (row.leftFraction / total) * (barW - 1);
          const rightW = (row.rightFraction / total) * (barW - 1);
          if (leftW > 0.5) {
            ctx.fillStyle = model.left.leadColor;
            ctx.beginPath();
            ctx.roundRect(barX, barY, leftW, barH, [3, 0, 0, 3]);
            ctx.fill();
          }
          if (rightW > 0.5) {
            ctx.fillStyle = model.right.leadColor;
            ctx.beginPath();
            ctx.roundRect(barX + leftW + 1, barY, rightW, barH, [0, 3, 3, 0]);
            ctx.fill();
          }
        }
        ctx.restore();
      }
    }
  }

  const right = layout.rightArea;
  const capCy = right.y + 10;
  ctx.save();
  ctx.textBaseline = 'middle';
  ctx.font = `600 13px ${theme.font}`;
  ctx.fillStyle = theme.text;
  ctx.textAlign = 'left';
  ctx.fillText(layout.measureLabel, right.x, capCy);
  const titleW = textWidth(ctx, layout.measureLabel);

  const lSum = model.left.totals[layout.selectedMeasure];
  const rSum = model.right.totals[layout.selectedMeasure];
  const lTxt = model.show === 'totals' ? lSum.totalText : lSum.warShareText;
  const rTxt = model.show === 'totals' ? rSum.totalText : rSum.warShareText;
  let rightLimit = right.x + right.width;

  if (layout.selectedMeasure !== 'participation') {
    ctx.font = `500 12px ${theme.font}`;
    const rW = textWidth(ctx, rTxt);
    const lW = textWidth(ctx, lTxt);
    ctx.textAlign = 'right';
    ctx.fillStyle = theme.text;
    ctx.fillText(rTxt, rightLimit, capCy);
    drawFlag(ctx, rightLimit - rW - 22, Math.round(capCy - 5.5), model.right.leadFlagArt, model.right.leadTag, theme, 16, 11);
    const lRight = rightLimit - rW - 34;
    ctx.fillText(lTxt, lRight, capCy);
    drawFlag(ctx, lRight - lW - 22, Math.round(capCy - 5.5), model.left.leadFlagArt, model.left.leadTag, theme, 16, 11);
    rightLimit = lRight - lW - 34;
  }

  const capX = right.x + titleW + 10;
  if (rightLimit - capX > 40) {
    ctx.font = `400 12px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.textAlign = 'left';
    ctx.fillText(fit(ctx, layout.caption, rightLimit - capX), capX, capCy);
  }
  ctx.restore();

  const bTop = layout.butterflyTop;
  const bHeight = Math.max(40, layout.height - bTop - 10);
  const cx = Math.floor(layout.centerX) + 0.5;
  ctx.save();
  ctx.strokeStyle = tokens.hairlineStrong;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(cx, bTop);
  ctx.lineTo(cx, bTop + bHeight);
  ctx.stroke();
  ctx.restore();

  ctx.save();
  ctx.beginPath();
  ctx.rect(right.x - 2, bTop, right.width + 4, bHeight);
  ctx.clip();

  const halfGap = 18;
  const leftBarRight = layout.centerX - halfGap;
  const rightBarLeft = layout.centerX + halfGap;
  const barH = 12;

  for (const row of layout.butterflyRows) {
    const sy = bTop + row.y - scrollTop;
    if (sy + row.height < bTop - 2 || sy > bTop + bHeight + 2) continue;
    const cy = sy + row.height / 2;

    if (row.left) {
      const m = row.left.member;
      const sideW = layout.centerX - halfGap - right.x;
      if (inspectedKey === m.key) drawRowHover(ctx, right.x, sy, sideW, row.height, theme, 0);
      drawFlag(ctx, right.x, Math.round(cy - 6), m.flagArt, m.tag, theme, 18, 12);
      ctx.save();
      ctx.font = `400 12px ${theme.font}`;
      ctx.fillStyle = theme.text;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillText(fit(ctx, m.name, Math.max(20, layout.nameColWidth - 26)), right.x + 24, cy);

      const barW = Math.max(2, row.left.barFraction * layout.barMaxWidth);
      const valX = leftBarRight;
      ctx.font = `500 12px ${theme.font}`;
      ctx.textAlign = 'right';
      ctx.fillText(row.left.valueText, valX, cy);
      ctx.restore();

      const bRight = leftBarRight - layout.valueColWidth;
      drawBar(ctx, bRight - barW, Math.round(cy - barH / 2), barW, barH, m.color, 'left');
      if (focusedKey === m.key) drawRowFocusRing(ctx, right.x, sy, sideW, row.height, theme, 0);
    }

    if (row.right) {
      const m = row.right.member;
      const sideX = rightBarLeft;
      const sideW = right.x + right.width - sideX;
      if (inspectedKey === m.key) drawRowHover(ctx, sideX, sy, sideW, row.height, theme, 0);
      ctx.save();
      ctx.font = `500 12px ${theme.font}`;
      ctx.fillStyle = theme.text;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillText(row.right.valueText, rightBarLeft, cy);

      const barW = Math.max(2, row.right.barFraction * layout.barMaxWidth);
      const bLeft = rightBarLeft + layout.valueColWidth;
      drawBar(ctx, bLeft, Math.round(cy - barH / 2), barW, barH, m.color, 'right');

      const flagX = right.x + right.width - 18;
      drawFlag(ctx, flagX, Math.round(cy - 6), m.flagArt, m.tag, theme, 18, 12);
      ctx.font = `400 12px ${theme.font}`;
      ctx.fillStyle = theme.text;
      ctx.textAlign = 'right';
      ctx.fillText(fit(ctx, m.name, Math.max(20, layout.nameColWidth - 26)), flagX - 6, cy);
      ctx.restore();
      if (focusedKey === m.key) drawRowFocusRing(ctx, sideX, sy, sideW, row.height, theme, 0);
    }
  }

  ctx.restore();
}

export function drawWarContribution(ctx: Canvas2D, input: WarContributionPaintInput): void {
  const { layout, theme } = input;
  ctx.save();
  ctx.fillStyle = theme.surface;
  ctx.fillRect(0, 0, layout.width, layout.height);
  if (layout.view === 'sides') {
    drawSidesView(ctx, input, layout);
  } else {
    drawMeasureView(ctx, input, layout);
  }
  ctx.restore();
}
