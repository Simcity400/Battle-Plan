import { LABEL_WEIGHT, VALUE_WEIGHT } from '../../charts/draw.ts';
import { ellipsise } from '../../charts/labels.ts';
import { readableColour } from '../../charts/palette.ts';
import { textWidth, type Canvas2D } from '../../charts/textWidth.ts';
import {
  drawArtSlot,
  drawFlagSlot,
  drawNameSlot,
  drawProgressRing,
  drawRowFocusRing,
  drawRowHairline,
  drawRowHover,
  drawRowSelected,
  drawScrollFade,
  drawThresholdMeter,
  drawTrackBar,
  resolveKitTokens,
} from '../../draw/kit.ts';
import { toArtSlotState, toFlagSlotState } from '../art/adapters.ts';
import type { FamilyTheme } from '../contract.ts';
import { drawHatchRect } from '../valueCanvas.ts';
import type {
  MeterChip,
  MeterFlag,
  MeterMatrix,
  MeterModel,
  MeterRow,
} from './model.ts';

const PAD_X = 14;
const PAD_Y = 10;
const SCROLL_FADE_PX = 24;

export interface MeterHitRect {
  readonly key: string;
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

interface PlacedRowItem {
  readonly kind: 'row';
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
  readonly row: MeterRow;
}

interface PlacedSectionHeader {
  readonly kind: 'header';
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
  readonly title: string;
  readonly count?: string;
  readonly rightCaption?: string;
  readonly strongRule?: boolean;
}

interface PlacedRule {
  readonly kind: 'rule';
  readonly x: number;
  readonly y: number;
  readonly width: number;
}

interface PlacedChip {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
  readonly chip: MeterChip;
}

interface PlacedChipLine {
  readonly kind: 'chip-line';
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
  readonly ownerWidth: number;
  readonly showOwnerName: boolean;
  readonly showTargetName: boolean;
  readonly showDate: boolean;
  readonly ownerFlag: MeterFlag;
  readonly ownerName: string;
  readonly chips: readonly PlacedChip[];
}

type PlacedBlock = PlacedSectionHeader | PlacedRowItem | PlacedRule | PlacedChipLine;

export interface MatrixGeometry {
  readonly matrix: MeterMatrix;
  readonly labelWidth: number;
  readonly showOwnerName: boolean;
  readonly colWidth: number;
  readonly headerHeight: number;
  readonly rowHeight: number;
  readonly showTotals: boolean;
  readonly totalX: number;
  readonly totalWidth: number;
}

export interface MeterLayout {
  readonly mode: MeterModel['layout'];
  readonly contentWidth: number;
  readonly contentHeight: number;
  readonly blocks: readonly PlacedBlock[];
  readonly matrix?: MatrixGeometry;
  readonly hits: readonly MeterHitRect[];
  readonly itemBands: ReadonlyMap<string, readonly [number, number]>;
}

function measureWithFont(ctx: Canvas2D, font: string, text: string): number {
  ctx.font = font;
  return textWidth(ctx, text);
}

export function layoutMeterModel(
  ctx: Canvas2D,
  model: MeterModel,
  viewportWidth: number,
  theme: FamilyTheme,
): MeterLayout {
  const width = Math.max(160, viewportWidth);
  const innerW = Math.max(120, width - PAD_X * 2);
  const blocks: PlacedBlock[] = [];
  const hits: MeterHitRect[] = [];
  const itemBands = new Map<string, readonly [number, number]>();

  if (model.layout === 'matrix' && model.matrix) {
    const matrix = model.matrix;
    const showOwnerName = innerW >= 420;
    const labelWidth = showOwnerName ? 150 : 28;
    const colWidth = 30;
    const headerHeight = 28;
    const rowHeight = 28;
    const showTotals = innerW >= 540;
    const totalWidth = showTotals ? 156 : 0;
    const colsTotalW = matrix.columns.length * colWidth;
    const totalX = PAD_X + labelWidth + colsTotalW + 12;
    const contentWidth = Math.max(
      width,
      PAD_X * 2 + labelWidth + colsTotalW + (showTotals ? 12 + totalWidth : 0),
    );
    const contentHeight = PAD_Y * 2 + headerHeight + matrix.rows.length * rowHeight;

    for (let r = 0; r < matrix.rows.length; r++) {
      const row = matrix.rows[r]!;
      const y = PAD_Y + headerHeight + r * rowHeight;
      for (let c = 0; c < row.cells.length; c++) {
        const cell = row.cells[c]!;
        if (cell.kind === 'diagonal' || cell.kind === 'none') continue;
        const x = PAD_X + labelWidth + c * colWidth;
        hits.push({ key: cell.key, x, y, width: colWidth, height: rowHeight });
        itemBands.set(cell.key, [y, rowHeight]);
      }
    }

    return {
      mode: 'matrix',
      contentWidth,
      contentHeight,
      blocks,
      matrix: {
        matrix,
        labelWidth,
        showOwnerName,
        colWidth,
        headerHeight,
        rowHeight,
        showTotals,
        totalX,
        totalWidth,
      },
      hits,
      itemBands,
    };
  }

  if (model.layout === 'columns') {
    const sections = model.sections ?? [];
    const twoCol = innerW >= 680 && sections.length > 1;
    const colGap = 24;
    const colW = twoCol ? Math.floor((innerW - colGap) / 2) : innerW;
    const colY = twoCol ? [PAD_Y, PAD_Y] : [PAD_Y];
    const rowH = colW < 420 ? 28 : 38;
    const headH = 24;
    const sectionGap = 14;

    for (const section of sections) {
      let colIdx = 0;
      if (twoCol && colY[1]! < colY[0]!) colIdx = 1;
      const x = PAD_X + (colIdx === 1 ? colW + colGap : 0);
      let y = colY[colIdx]!;
      blocks.push({
        kind: 'header',
        x,
        y,
        width: colW,
        height: headH,
        title: section.title,
        count: section.count,
        rightCaption: section.rightCaption,
        strongRule: true,
      });
      y += headH + 2;
      for (const row of section.rows ?? []) {
        blocks.push({ kind: 'row', x, y, width: colW, height: rowH, row });
        hits.push({ key: row.key, x, y, width: colW, height: rowH });
        itemBands.set(row.key, [y, rowH]);
        y += rowH;
      }
      colY[colIdx] = y + sectionGap;
    }

    const maxBottom = Math.max(...colY, PAD_Y);
    return {
      mode: 'columns',
      contentWidth: width,
      contentHeight: Math.max(PAD_Y * 2, maxBottom - sectionGap + PAD_Y),
      blocks,
      hits,
      itemBands,
    };
  }

  if (model.layout === 'ciphers') {
    const sections = model.sections ?? [];
    let y = PAD_Y;
    const nameFont = `${LABEL_WEIGHT} 12px ${theme.font}`;
    const dateFont = `400 12px ${theme.font}`;
    const showOwnerName = innerW >= 420;
    const ownerW = showOwnerName ? 132 : 28;
    const showTargetName = innerW >= 1050;
    const showDate = innerW >= 380;

    for (let s = 0; s < sections.length; s++) {
      const section = sections[s]!;
      const hasRows = (section.rows?.length ?? 0) > 0;
      const hasChips = (section.chipLines?.length ?? 0) > 0;
      if (!hasRows && !hasChips) continue;

      if (blocks.length > 0) {
        y += 6;
        blocks.push({ kind: 'rule', x: PAD_X, y, width: innerW });
        y += 8;
      }

      const headH = 20;
      blocks.push({
        kind: 'header',
        x: PAD_X,
        y,
        width: innerW,
        height: headH,
        title: section.title,
        count: section.count,
        rightCaption: section.rightCaption,
      });
      y += headH + 4;

      if (section.rows) {
        const rowH = 23;
        for (const row of section.rows) {
          blocks.push({ kind: 'row', x: PAD_X, y, width: innerW, height: rowH, row });
          hits.push({ key: row.key, x: PAD_X, y, width: innerW, height: rowH });
          itemBands.set(row.key, [y, rowH]);
          y += rowH + 2;
        }
      }

      if (section.chipLines) {
        const lineH = 22;
        const chipGap = 14;
        const chipStartX = PAD_X + ownerW + 8;
        const maxChipX = PAD_X + innerW;
        for (const line of section.chipLines) {
          let cx = chipStartX;
          let cy = y;
          const placedChips: PlacedChip[] = [];
          for (const chip of line.chips) {
            let chipW = 18;
            if (showTargetName) {
              chipW += 5 + Math.ceil(measureWithFont(ctx, nameFont, chip.targetName));
            }
            if (showDate) {
              chipW += 5 + (chip.dateText !== null ? Math.ceil(measureWithFont(ctx, dateFont, chip.dateText)) : 12);
            }
            if (cx > chipStartX && cx + chipW > maxChipX) {
              cx = chipStartX;
              cy += lineH;
            }
            placedChips.push({ x: cx, y: cy, width: chipW, height: lineH, chip });
            hits.push({ key: chip.key, x: cx - 3, y: cy, width: chipW + 6, height: lineH });
            itemBands.set(chip.key, [cy, lineH]);
            cx += chipW + chipGap;
          }
          const blockH = Math.max(lineH, cy - y + lineH);
          blocks.push({
            kind: 'chip-line',
            x: PAD_X,
            y,
            width: innerW,
            height: blockH,
            ownerWidth: ownerW,
            showOwnerName,
            showTargetName,
            showDate,
            ownerFlag: line.ownerFlag,
            ownerName: line.ownerName,
            chips: placedChips,
          });
          y += blockH + 3;
        }
      }
    }

    return {
      mode: 'ciphers',
      contentWidth: width,
      contentHeight: Math.max(PAD_Y * 2, y + PAD_Y),
      blocks,
      hits,
      itemBands,
    };
  }

  const sections = model.sections ?? [];
  let y = PAD_Y;
  const rowH = 36;
  for (let s = 0; s < sections.length; s++) {
    const section = sections[s]!;
    if (s > 0) y += 10;
    const headH = 22;
    blocks.push({
      kind: 'header',
      x: PAD_X,
      y,
      width: innerW,
      height: headH,
      title: section.title,
      count: section.count,
      rightCaption: section.rightCaption,
    });
    y += headH + 4;
    for (const row of section.rows ?? []) {
      blocks.push({ kind: 'row', x: PAD_X, y, width: innerW, height: rowH, row });
      hits.push({ key: row.key, x: PAD_X, y, width: innerW, height: rowH });
      itemBands.set(row.key, [y, rowH]);
      y += rowH;
    }
  }

  return {
    mode: 'rows',
    contentWidth: width,
    contentHeight: Math.max(PAD_Y * 2, y + PAD_Y),
    blocks,
    hits,
    itemBands,
  };
}

function resolveBarColor(rawColor: string | undefined, theme: FamilyTheme): string {
  if (!rawColor) return theme.accent;
  return readableColour(rawColor, theme.surface);
}

function drawSectionHeaderBlock(
  ctx: Canvas2D,
  block: PlacedSectionHeader,
  theme: FamilyTheme,
): void {
  const midY = block.y + block.height / 2;
  ctx.save();
  ctx.textBaseline = 'middle';
  ctx.textAlign = 'left';
  ctx.font = `600 12px ${theme.font}`;
  ctx.fillStyle = theme.text;
  let rightLimit = block.x + block.width;
  if (block.rightCaption) {
    ctx.font = `400 11px ${theme.font}`;
    const capW = textWidth(ctx, block.rightCaption);
    ctx.fillStyle = theme.muted;
    ctx.textAlign = 'right';
    ctx.fillText(block.rightCaption, block.x + block.width, midY);
    rightLimit -= capW + 10;
    ctx.textAlign = 'left';
  }
  ctx.font = `600 12px ${theme.font}`;
  ctx.fillStyle = theme.text;
  const countW = block.count ? measureWithFont(ctx, `400 11.5px ${theme.font}`, block.count) + 8 : 0;
  const maxTitleW = Math.max(24, rightLimit - block.x - countW);
  const titleText = ellipsise(block.title, maxTitleW, s => textWidth(ctx, s));
  ctx.fillText(titleText, block.x, midY);
  if (block.count) {
    const drawnTitleW = textWidth(ctx, titleText);
    ctx.font = `400 11.5px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.fillText(block.count, block.x + drawnTitleW + 8, midY);
  }
  ctx.restore();
  if (block.strongRule) {
    drawRowHairline(ctx, block.x, block.x + block.width, block.y + block.height, theme, 'strong');
  }
}

function drawStandardMeterRow(
  ctx: Canvas2D,
  block: PlacedRowItem,
  theme: FamilyTheme,
  inspectedKey: string | null,
  selectedKey: string | null,
  focusedKey: string | null,
): void {
  const { x, y, width, height, row } = block;
  const midY = y + height / 2;
  if (selectedKey === row.key) drawRowSelected(ctx, x - 4, y, width + 8, height, theme, 0);
  else if (inspectedKey === row.key) drawRowHover(ctx, x - 4, y, width + 8, height, theme, 0);
  if (focusedKey === row.key) drawRowFocusRing(ctx, x - 4, y, width + 8, height, theme, 0);

  let cursorX = x;
  if (row.art) {
    const size = 30;
    const artY = Math.round(midY - size / 2);
    if (row.art.idle) {
      const tokens = resolveKitTokens(theme);
      ctx.save();
      ctx.fillStyle = tokens.headerBand;
      ctx.beginPath();
      ctx.roundRect(cursorX, artY, size, size, row.art.radius ?? 4);
      ctx.fill();
      ctx.fillStyle = theme.muted;
      ctx.fillRect(cursorX + 11, artY + 10, 2.5, 10);
      ctx.fillRect(cursorX + 16.5, artY + 10, 2.5, 10);
      ctx.restore();
    } else {
      drawArtSlot(ctx, cursorX, artY, size, size, toArtSlotState(row.art.result, row.art.resolvedName), theme, { radius: row.art.radius ?? 4 });
    }
    cursorX += size + 10;
  }

  const showDays = width >= 460;
  const daysW = showDays ? 88 : 0;
  const pctW = 44;
  const barW = row.bar.kind === 'none' && row.muted
    ? 0
    : Math.max(60, Math.min(240, Math.round(width * 0.28)));
  const rightReserved = (barW > 0 ? barW + 10 : 0) + (row.figure !== undefined || row.figureMissing ? pctW + 10 : 0) + (showDays ? daysW + 10 : 0);
  const nameW = Math.max(60, x + width - cursorX - rightReserved);

  const showSubtitle = width >= 440 && !!row.subtitle;
  if (showSubtitle) {
    ctx.save();
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    ctx.font = `${LABEL_WEIGHT} 13px ${theme.font}`;
    ctx.fillStyle = row.muted ? theme.muted : theme.text;
    ctx.fillText(ellipsise(row.name, nameW, s => textWidth(ctx, s)), cursorX, y + 12.5);
    ctx.font = `400 11px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.fillText(ellipsise(row.subtitle!, nameW, s => textWidth(ctx, s)), cursorX, y + 26);
    ctx.restore();
  } else {
    drawNameSlot(ctx, row.name, cursorX, y, nameW, height, theme, {
      fontSize: 13,
      weight: LABEL_WEIGHT,
      color: row.muted ? theme.muted : theme.text,
      maxLines: 2,
    });
  }

  let rightX = x + width;
  if (showDays && row.secondary) {
    ctx.save();
    ctx.font = `400 12px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.textAlign = 'right';
    ctx.textBaseline = 'middle';
    ctx.fillText(ellipsise(row.secondary, daysW, s => textWidth(ctx, s)), rightX, midY);
    ctx.restore();
  }
  if (showDays) rightX -= daysW + 10;

  if (row.figureMissing) {
    const label = row.figureMissingLabel;
    if (label) {
      ctx.save();
      ctx.font = `400 11.5px ${theme.font}`;
      const lw = textWidth(ctx, label);
      ctx.fillStyle = theme.muted;
      ctx.textAlign = 'right';
      ctx.textBaseline = 'middle';
      ctx.fillText(label, rightX, midY);
      drawHatchRect(ctx, rightX - lw - 16, Math.round(midY - 5), 10, 10, 2, theme);
      ctx.restore();
    } else {
      drawHatchRect(ctx, rightX - 11, Math.round(midY - 5.5), 11, 11, 2, theme);
    }
    rightX -= pctW + 10;
  } else if (row.figure !== undefined) {
    ctx.save();
    ctx.font = `${VALUE_WEIGHT} 12px ${theme.font}`;
    ctx.fillStyle = row.muted ? theme.muted : theme.text;
    ctx.textAlign = 'right';
    ctx.textBaseline = 'middle';
    ctx.fillText(row.figure, rightX, midY);
    ctx.restore();
    rightX -= pctW + 10;
  }

  if (barW > 0) {
    const barX = rightX - barW;
    if (row.bar.kind === 'track') {
      const h = row.bar.height ?? 6;
      if (row.bar.fraction === null) {
        drawHatchRect(ctx, barX, Math.round(midY - h / 2), barW, h, 3, theme);
      } else {
        drawTrackBar(ctx, barX, midY, barW, h, row.bar.fraction, resolveBarColor(row.bar.color, theme), theme);
      }
    } else if (row.bar.kind === 'threshold') {
      drawThresholdMeter(
        ctx,
        barX,
        midY,
        barW,
        row.bar.height ?? 7,
        row.bar.fraction,
        row.bar.ticks,
        resolveBarColor(row.bar.color, theme),
        theme,
      );
    }
  }

  drawRowHairline(ctx, x, x + width, y + height, theme);
}

function drawColumnMeterRow(
  ctx: Canvas2D,
  block: PlacedRowItem,
  theme: FamilyTheme,
  inspectedKey: string | null,
  selectedKey: string | null,
  focusedKey: string | null,
): void {
  const { x, y, width, height, row } = block;
  const midY = y + height / 2;
  if (selectedKey === row.key) drawRowSelected(ctx, x - 4, y, width + 8, height, theme, 0);
  else if (inspectedKey === row.key) drawRowHover(ctx, x - 4, y, width + 8, height, theme, 0);
  if (focusedKey === row.key) drawRowFocusRing(ctx, x - 4, y, width + 8, height, theme, 0);

  const narrow = width < 420;
  const valW = narrow ? 44 : 136;
  const nameW = Math.max(84, Math.min(200, Math.round(width * 0.36)));
  const barX = x + nameW + 10;
  const barW = Math.max(40, width - nameW - valW - 20);

  if (!narrow && row.subtitle) {
    ctx.save();
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    ctx.font = `${LABEL_WEIGHT} 12px ${theme.font}`;
    ctx.fillStyle = row.muted ? theme.muted : theme.text;
    ctx.fillText(ellipsise(row.name, nameW, s => textWidth(ctx, s)), x, y + 13);
    ctx.font = `400 11px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.fillText(ellipsise(row.subtitle, nameW, s => textWidth(ctx, s)), x, y + 27);
    ctx.restore();
  } else {
    drawNameSlot(ctx, row.name, x, y, nameW, height, theme, {
      fontSize: 12,
      weight: LABEL_WEIGHT,
      color: row.muted ? theme.muted : theme.text,
      maxLines: 1,
    });
  }

  if (row.bar.kind === 'threshold') {
    drawThresholdMeter(
      ctx,
      barX,
      midY,
      barW,
      row.bar.height ?? 7,
      row.bar.fraction,
      row.bar.ticks,
      resolveBarColor(row.bar.color, theme),
      theme,
    );
  } else if (row.bar.kind === 'track') {
    const h = row.bar.height ?? 7;
    if (row.bar.fraction === null) {
      drawHatchRect(ctx, barX, Math.round(midY - h / 2), barW, h, 3, theme);
    } else {
      drawTrackBar(ctx, barX, midY, barW, h, row.bar.fraction, resolveBarColor(row.bar.color, theme), theme);
    }
  } else if (row.bar.dash) {
    ctx.save();
    ctx.font = `400 12px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText('\u2014', barX + barW / 2, midY);
    ctx.restore();
  }

  const rightX = x + width;
  if (narrow) {
    if (row.figureMissing) {
      drawHatchRect(ctx, rightX - 11, Math.round(midY - 5.5), 11, 11, 2, theme);
    } else {
      const compact = row.compactFigure ?? row.figure ?? row.secondary ?? '';
      if (compact) {
        ctx.save();
        ctx.font = `${VALUE_WEIGHT} 12px ${theme.font}`;
        ctx.fillStyle = row.muted ? theme.muted : theme.text;
        ctx.textAlign = 'right';
        ctx.textBaseline = 'middle';
        ctx.fillText(ellipsise(compact, valW, s => textWidth(ctx, s)), rightX, midY);
        ctx.restore();
      }
    }
  } else if (row.figureMissing) {
    const label = row.figureMissingLabel || 'Not recorded';
    ctx.save();
    ctx.font = `400 11.5px ${theme.font}`;
    const lw = textWidth(ctx, label);
    ctx.fillStyle = theme.muted;
    ctx.textAlign = 'right';
    ctx.textBaseline = 'middle';
    ctx.fillText(label, rightX, midY);
    drawHatchRect(ctx, rightX - lw - 16, Math.round(midY - 5), 10, 10, 2, theme);
    ctx.restore();
  } else {
    const dailyW = 58;
    const masteryRight = row.secondary ? rightX - dailyW - 6 : rightX;
    if (row.secondary) {
      ctx.save();
      ctx.font = `400 11px ${theme.font}`;
      ctx.fillStyle = theme.muted;
      ctx.textAlign = 'right';
      ctx.textBaseline = 'middle';
      ctx.fillText(ellipsise(row.secondary, row.figure ? dailyW : valW, s => textWidth(ctx, s)), rightX, midY);
      ctx.restore();
    }
    if (row.figure) {
      ctx.save();
      ctx.font = `600 12px ${theme.font}`;
      ctx.fillStyle = row.muted ? theme.muted : theme.text;
      ctx.textAlign = 'right';
      ctx.textBaseline = 'middle';
      ctx.fillText(ellipsise(row.figure, valW - (row.secondary ? dailyW + 6 : 0), s => textWidth(ctx, s)), masteryRight, midY);
      ctx.restore();
    }
  }

  drawRowHairline(ctx, x, x + width, y + height, theme);
}

function drawCipherMeterRow(
  ctx: Canvas2D,
  block: PlacedRowItem,
  theme: FamilyTheme,
  inspectedKey: string | null,
  selectedKey: string | null,
  focusedKey: string | null,
): void {
  const { x, y, width, height, row } = block;
  const midY = y + height / 2;
  if (selectedKey === row.key) drawRowSelected(ctx, x - 4, y, width + 8, height, theme, 0);
  else if (inspectedKey === row.key) drawRowHover(ctx, x - 4, y, width + 8, height, theme, 0);
  if (focusedKey === row.key) drawRowFocusRing(ctx, x - 4, y, width + 8, height, theme, 0);

  const wide = width >= 500;
  const pairW = wide ? Math.min(300, Math.round(width * 0.42)) : Math.min(170, Math.round(width * 0.48));
  const pctW = 44;
  const barX = x + pairW + 10;
  const barW = Math.max(40, width - pairW - pctW - 20);

  if (row.pair) {
    const flagY = Math.round(midY - 6.5);
    let cx = x;
    drawFlagSlot(ctx, cx, flagY, toFlagSlotState(row.pair.ownerFlag.result, row.pair.ownerFlag.tag), theme, { width: 20, height: 13 });
    cx += 20;
    if (wide) {
      cx += 6;
      const eachNameW = Math.max(36, Math.floor((pairW - 68) / 2));
      ctx.save();
      ctx.font = `400 12px ${theme.font}`;
      ctx.fillStyle = theme.muted;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      const ownerShown = ellipsise(row.pair.ownerName, eachNameW, s => textWidth(ctx, s));
      ctx.fillText(ownerShown, cx, midY);
      cx += Math.min(eachNameW, textWidth(ctx, ownerShown)) + 6;
      ctx.restore();
    } else {
      cx += 5;
    }
    ctx.save();
    ctx.font = `400 11px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    ctx.fillText('\u2192', cx, midY);
    cx += textWidth(ctx, '\u2192') + 5;
    ctx.restore();

    drawFlagSlot(ctx, cx, flagY, toFlagSlotState(row.pair.targetFlag.result, row.pair.targetFlag.tag), theme, { width: 20, height: 13 });
    cx += 26;
    const remW = Math.max(24, x + pairW - cx);
    ctx.save();
    ctx.font = `${LABEL_WEIGHT} 12px ${theme.font}`;
    ctx.fillStyle = theme.text;
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    ctx.fillText(ellipsise(row.pair.targetName, remW, s => textWidth(ctx, s)), cx, midY);
    ctx.restore();
  }

  if (row.bar.kind === 'track') {
    const h = row.bar.height ?? 8;
    if (row.bar.fraction === null) {
      drawHatchRect(ctx, barX, Math.round(midY - h / 2), barW, h, 3, theme);
    } else {
      drawTrackBar(ctx, barX, midY, barW, h, row.bar.fraction, resolveBarColor(row.bar.color, theme), theme);
    }
  }

  const rightX = x + width;
  if (row.figureMissing) {
    drawHatchRect(ctx, rightX - 11, Math.round(midY - 5.5), 11, 11, 2, theme);
  } else if (row.figure !== undefined) {
    ctx.save();
    ctx.font = `${VALUE_WEIGHT} 12px ${theme.font}`;
    ctx.fillStyle = theme.text;
    ctx.textAlign = 'right';
    ctx.textBaseline = 'middle';
    ctx.fillText(row.figure, rightX, midY);
    ctx.restore();
  }
}

function drawChipLineBlock(
  ctx: Canvas2D,
  block: PlacedChipLine,
  theme: FamilyTheme,
  inspectedKey: string | null,
  selectedKey: string | null,
  focusedKey: string | null,
): void {
  const firstMidY = block.y + 11;
  drawFlagSlot(ctx, block.x, Math.round(firstMidY - 6.5), toFlagSlotState(block.ownerFlag.result, block.ownerFlag.tag), theme, { width: 20, height: 13 });
  if (block.showOwnerName) {
    const maxW = Math.max(24, block.ownerWidth - 26);
    ctx.save();
    ctx.font = `${LABEL_WEIGHT} 12px ${theme.font}`;
    ctx.fillStyle = theme.text;
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    ctx.fillText(ellipsise(block.ownerName, maxW, s => textWidth(ctx, s)), block.x + 26, firstMidY);
    ctx.restore();
  }

  for (const placed of block.chips) {
    const { chip } = placed;
    const midY = placed.y + placed.height / 2;
    if (selectedKey === chip.key) drawRowSelected(ctx, placed.x - 4, placed.y, placed.width + 8, placed.height, theme, 0);
    else if (inspectedKey === chip.key) drawRowHover(ctx, placed.x - 4, placed.y, placed.width + 8, placed.height, theme, 0);
    if (focusedKey === chip.key) drawRowFocusRing(ctx, placed.x - 4, placed.y, placed.width + 8, placed.height, theme, 0);

    let cx = placed.x;
    drawFlagSlot(ctx, cx, Math.round(midY - 6), toFlagSlotState(chip.targetFlag.result, chip.targetFlag.tag), theme, { width: 18, height: 12 });
    cx += 18;
    if (block.showTargetName) {
      cx += 5;
      ctx.save();
      ctx.font = `${LABEL_WEIGHT} 12px ${theme.font}`;
      ctx.fillStyle = theme.text;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillText(chip.targetName, cx, midY);
      cx += textWidth(ctx, chip.targetName);
      ctx.restore();
    }
    if (block.showDate) {
      cx += 5;
      if (chip.dateText !== null) {
        ctx.save();
        ctx.font = `400 12px ${theme.font}`;
        ctx.fillStyle = theme.muted;
        ctx.textAlign = 'left';
        ctx.textBaseline = 'middle';
        ctx.fillText(chip.dateText, cx, midY);
        ctx.restore();
      } else {
        drawHatchRect(ctx, cx, Math.round(midY - 5.5), 11, 11, 2, theme);
      }
    }
  }
}

function drawMatrixBody(
  ctx: Canvas2D,
  geom: MatrixGeometry,
  viewportWidth: number,
  viewportHeight: number,
  scrollLeft: number,
  scrollTop: number,
  theme: FamilyTheme,
  inspectedKey: string | null,
  selectedKey: string | null,
  focusedKey: string | null,
): void {
  const { matrix, labelWidth, showOwnerName, colWidth, headerHeight, rowHeight, showTotals, totalX, totalWidth } = geom;
  const stickyLeft = PAD_X + labelWidth;
  const stickyTop = PAD_Y + headerHeight;
  const tokens = resolveKitTokens(theme);

  ctx.save();
  ctx.beginPath();
  ctx.rect(stickyLeft, stickyTop, Math.max(0, viewportWidth - stickyLeft), Math.max(0, viewportHeight - stickyTop));
  ctx.clip();

  for (let r = 0; r < matrix.rows.length; r++) {
    const row = matrix.rows[r]!;
    const y = stickyTop + r * rowHeight - scrollTop;
    if (y + rowHeight < stickyTop || y > viewportHeight) continue;
    const cy = y + rowHeight / 2;
    drawRowHairline(ctx, stickyLeft, viewportWidth, y + rowHeight, theme);

    for (let c = 0; c < row.cells.length; c++) {
      const cell = row.cells[c]!;
      const x = stickyLeft + c * colWidth - scrollLeft;
      if (x + colWidth < stickyLeft || x > viewportWidth) continue;
      const cx = x + colWidth / 2;
      if (selectedKey === cell.key) drawRowSelected(ctx, x + 2, y + 2, colWidth - 4, rowHeight - 4, theme, 0);
      else if (inspectedKey === cell.key) drawRowHover(ctx, x + 2, y + 2, colWidth - 4, rowHeight - 4, theme, 0);
      if (focusedKey === cell.key) drawRowFocusRing(ctx, x + 2, y + 2, colWidth - 4, rowHeight - 4, theme, 0);

      if (cell.kind === 'diagonal') {
        ctx.save();
        ctx.strokeStyle = tokens.bandDivider;
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.moveTo(cx - 3.5, cy + 3.5);
        ctx.lineTo(cx + 3.5, cy - 3.5);
        ctx.stroke();
        ctx.restore();
      } else if (cell.kind === 'broken') {
        ctx.save();
        ctx.fillStyle = resolveBarColor(cell.color, theme);
        ctx.beginPath();
        ctx.roundRect(cx - 7, cy - 7, 14, 14, 3);
        ctx.fill();
        ctx.restore();
      } else if (cell.kind === 'progress') {
        drawProgressRing(ctx, cx, cy, 6.2, cell.fraction ?? 0, resolveBarColor(cell.color, theme), theme, 2.4);
      } else if (cell.kind === 'missing') {
        drawHatchRect(ctx, cx - 7, cy - 7, 14, 14, 2.5, theme);
      } else {
        ctx.save();
        ctx.fillStyle = tokens.bandDivider;
        ctx.beginPath();
        ctx.arc(cx, cy, 1.3, 0, Math.PI * 2);
        ctx.fill();
        ctx.restore();
      }
    }

    if (showTotals && row.totalText) {
      const tx = totalX - scrollLeft;
      if (tx < viewportWidth) {
        ctx.save();
        ctx.font = `400 11px ${theme.font}`;
        ctx.fillStyle = theme.muted;
        ctx.textAlign = 'left';
        ctx.textBaseline = 'middle';
        ctx.fillText(ellipsise(row.totalText, totalWidth, s => textWidth(ctx, s)), tx, cy);
        ctx.restore();
      }
    }
  }
  ctx.restore();

  ctx.save();
  ctx.fillStyle = theme.surface;
  ctx.fillRect(0, stickyTop, stickyLeft, Math.max(0, viewportHeight - stickyTop));
  ctx.beginPath();
  ctx.rect(0, stickyTop, stickyLeft, Math.max(0, viewportHeight - stickyTop));
  ctx.clip();
  for (let r = 0; r < matrix.rows.length; r++) {
    const row = matrix.rows[r]!;
    const y = stickyTop + r * rowHeight - scrollTop;
    if (y + rowHeight < stickyTop || y > viewportHeight) continue;
    const cy = y + rowHeight / 2;
    drawFlagSlot(ctx, PAD_X, Math.round(cy - 6.5), toFlagSlotState(row.ownerFlag.result, row.ownerFlag.tag), theme, { width: 20, height: 13 });
    if (showOwnerName) {
      const maxW = Math.max(24, labelWidth - 28);
      ctx.save();
      ctx.font = `${LABEL_WEIGHT} 12px ${theme.font}`;
      ctx.fillStyle = theme.text;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillText(ellipsise(row.ownerName, maxW, s => textWidth(ctx, s)), PAD_X + 26, cy);
      ctx.restore();
    }
    drawRowHairline(ctx, PAD_X, stickyLeft, y + rowHeight, theme);
  }
  ctx.restore();

  ctx.save();
  ctx.fillStyle = theme.surface;
  ctx.fillRect(stickyLeft, 0, Math.max(0, viewportWidth - stickyLeft), stickyTop);
  ctx.beginPath();
  ctx.rect(stickyLeft, 0, Math.max(0, viewportWidth - stickyLeft), stickyTop);
  ctx.clip();
  for (let c = 0; c < matrix.columns.length; c++) {
    const col = matrix.columns[c]!;
    const x = stickyLeft + c * colWidth - scrollLeft;
    if (x + colWidth < stickyLeft || x > viewportWidth) continue;
    drawFlagSlot(
      ctx,
      Math.round(x + (colWidth - 18) / 2),
      Math.round(PAD_Y + (headerHeight - 12) / 2),
      toFlagSlotState(col.flag.result, col.flag.tag),
      theme,
      { width: 18, height: 12 },
    );
  }
  drawRowHairline(ctx, stickyLeft, viewportWidth, stickyTop, theme, 'strong');
  ctx.restore();

  ctx.save();
  ctx.fillStyle = theme.surface;
  ctx.fillRect(0, 0, stickyLeft, stickyTop);
  ctx.font = `500 11px ${theme.font}`;
  ctx.fillStyle = theme.muted;
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  ctx.fillText(matrix.cornerLabel, PAD_X, PAD_Y + headerHeight / 2);
  drawRowHairline(ctx, PAD_X, stickyLeft, stickyTop, theme, 'strong');
  ctx.restore();
}

export interface DrawMeterInput {
  readonly layout: MeterLayout;
  readonly width: number;
  readonly height: number;
  readonly scrollLeft: number;
  readonly scrollTop: number;
  readonly theme: FamilyTheme;
  readonly inspectedKey: string | null;
  readonly selectedKey: string | null;
  readonly focusedKey: string | null;
}

export function drawMeterModel(ctx: Canvas2D, input: DrawMeterInput): void {
  const { layout, width, height, scrollLeft, scrollTop, theme, inspectedKey, selectedKey, focusedKey } = input;
  ctx.save();
  ctx.fillStyle = theme.surface;
  ctx.fillRect(0, 0, width, height);

  if (layout.mode === 'matrix' && layout.matrix) {
    drawMatrixBody(ctx, layout.matrix, width, height, scrollLeft, scrollTop, theme, inspectedKey, selectedKey, focusedKey);
  } else {
    ctx.save();
    ctx.translate(-scrollLeft, -scrollTop);
    const visTop = scrollTop - 40;
    const visBottom = scrollTop + height + 40;

    for (const block of layout.blocks) {
      if (block.kind === 'rule') {
        if (block.y >= visTop && block.y <= visBottom) {
          drawRowHairline(ctx, block.x, block.x + block.width, block.y, theme);
        }
        continue;
      }
      if (block.y + block.height < visTop || block.y > visBottom) continue;

      if (block.kind === 'header') {
        drawSectionHeaderBlock(ctx, block, theme);
      } else if (block.kind === 'chip-line') {
        drawChipLineBlock(ctx, block, theme, inspectedKey, selectedKey, focusedKey);
      } else if (layout.mode === 'columns') {
        drawColumnMeterRow(ctx, block, theme, inspectedKey, selectedKey, focusedKey);
      } else if (layout.mode === 'ciphers') {
        drawCipherMeterRow(ctx, block, theme, inspectedKey, selectedKey, focusedKey);
      } else {
        drawStandardMeterRow(ctx, block, theme, inspectedKey, selectedKey, focusedKey);
      }
    }
    ctx.restore();
  }

  if (scrollTop + height < layout.contentHeight - 2) {
    drawScrollFade(ctx, 0, height - SCROLL_FADE_PX, width, SCROLL_FADE_PX, theme, 'bottom');
  }
  if (scrollLeft + width < layout.contentWidth - 2) {
    drawScrollFade(ctx, width - SCROLL_FADE_PX, 0, SCROLL_FADE_PX, height, theme, 'right');
  }
  ctx.restore();
}

export function hitMeterModel(
  layout: MeterLayout,
  viewportX: number,
  viewportY: number,
  scrollLeft: number,
  scrollTop: number,
): string | null {
  if (layout.mode === 'matrix' && layout.matrix) {
    const stickyLeft = PAD_X + layout.matrix.labelWidth;
    const stickyTop = PAD_Y + layout.matrix.headerHeight;
    if (viewportX < stickyLeft || viewportY < stickyTop) return null;
  }
  const lx = viewportX + scrollLeft;
  const ly = viewportY + scrollTop;
  for (let i = 0; i < layout.hits.length; i++) {
    const h = layout.hits[i]!;
    if (lx >= h.x && lx <= h.x + h.width && ly >= h.y && ly <= h.y + h.height) {
      return `row:${h.key}`;
    }
  }
  return null;
}
