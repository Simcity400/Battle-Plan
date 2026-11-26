import { ellipsiseCanvasText, measureCanvasText, type Canvas2D } from '../../charts/textWidth.ts';
import {
  DATE_COLUMN_WIDTH,
  drawDateColumn,
  drawMissingMark,
  drawRowFocusRing,
  drawRowHairline,
  drawRowHover,
  drawRowSelected,
  drawScrollFade,
  drawShareBar,
  drawStatusMark,
  drawThresholdMeter,
  drawTrackBar,
  resolveKitTokens,
} from '../../draw/kit.ts';
import { drawMediaSlot } from '../art/adapters.ts';
import type { LaneArtCache } from '../art/laneCache.ts';
import { collectArtUrls, type ArtUrlInput } from '../art/provider.ts';
import type { FamilyTheme } from '../contract.ts';
import { formatValueState } from '../valueFormat.ts';
import {
  CARD_FADE,
  CARD_FLAG_HEIGHT,
  CARD_FLAG_WIDTH,
  CARD_GAP,
  CARD_GUTTER,
  CARD_HERO_HEIGHT,
  CARD_METER_FIGURE,
  CARD_METER_NAME,
  CARD_PAD_X,
  cardRowAt,
  visibleCardRows,
  type CardLayout,
} from './layout.ts';
import { cardTitle, type CardFigure, type CardHeroRow, type CardItemRow, type CardListRow, type CardMeter, type CardMeterRow, type CardModel, type CardSectionRow } from './model.ts';

export interface DrawCardsInput {
  readonly model: CardModel;
  readonly layout: CardLayout;
  readonly theme: FamilyTheme;
  readonly scrollTop: number;
  readonly artCache?: LaneArtCache;
  readonly inspectedKey?: string | null;
  readonly selectedKey?: string | null;
  readonly focusedKey?: string | null;
}

const MUTED_ROW_ALPHA = 0.72;

function line(ctx: Canvas2D, text: string, x: number, y: number, width: number, font: string, color: string, align: CanvasTextAlign = 'left'): number {
  if (!text || width <= 0) return 0;
  ctx.font = font;
  ctx.fillStyle = color;
  ctx.textAlign = align;
  ctx.textBaseline = 'middle';
  const shown = ellipsiseCanvasText(ctx, text, width, font);
  ctx.fillText(shown, align === 'right' ? x + width : x, y);
  return measureCanvasText(ctx, shown, font);
}

function meterFraction(meter: Pick<CardMeter, 'fraction'>): number | null {
  const v = meter.fraction;
  return v.kind === 'known' || v.kind === 'partial' ? Number(v.value) : v.kind === 'zero' ? 0 : null;
}

function drawMeter(ctx: Canvas2D, meter: CardMeter, x: number, middleY: number, width: number, height: number, theme: FamilyTheme): void {
  const fraction = meterFraction(meter);
  if (fraction === null) {
    drawMissingMark(ctx, x, middleY, width, theme, { figure: 'Progress' });
    return;
  }
  drawTrackBar(ctx, x, middleY, width, height, fraction, theme.accent, theme);
}

function drawFigure(ctx: Canvas2D, figure: CardFigure, x: number, middleY: number, width: number, align: 'left' | 'right', theme: FamilyTheme): void {
  const formatted = formatValueState(figure.value);
  if (formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable') {
    drawMissingMark(ctx, x, middleY, width, theme, { figure: figure.figure, align, notApplicable: formatted.kind === 'notApplicable' });
  } else {
    line(ctx, formatted.text, x, middleY, width, `400 ${figure.muted ? 11 : 12}px ${theme.font}`, figure.muted ? theme.muted : theme.text, align);
  }
}

function drawGutter(ctx: Canvas2D, label: string | undefined, middleY: number, theme: FamilyTheme): number {
  if (label) line(ctx, label, CARD_PAD_X, middleY, CARD_GUTTER, `400 12px ${theme.font}`, theme.muted);
  return CARD_PAD_X + CARD_GUTTER + 12;
}

function drawMeterRow(ctx: Canvas2D, row: CardMeterRow, top: number, height: number, width: number, theme: FamilyTheme): void {
  const middleY = top + height / 2;
  let x = drawGutter(ctx, row.label, middleY, theme);
  let right = width - CARD_PAD_X;
  if (row.figure) {
    drawFigure(ctx, row.figure, right - CARD_METER_FIGURE, middleY, CARD_METER_FIGURE, 'right', theme);
    right -= CARD_METER_FIGURE + CARD_GAP;
  }
  if (row.name) {
    const nameWidth = Math.min(CARD_METER_NAME, Math.round((right - x) * 0.4));
    line(ctx, row.name, x, middleY, nameWidth, `400 12.5px ${theme.font}`, theme.text);
    x += nameWidth + 10;
  }
  const barWidth = Math.max(0, right - x);
  if (row.bar.kind === 'share') {
    const segments = row.bar.segments.map(s => ({ share: s.share, color: s.tone === 'positive' ? theme.green : theme.red, label: s.label, shortLabel: s.shortLabel, labelAlign: s.labelAlign }));
    drawShareBar(ctx, x, Math.round(middleY - 7), barWidth, 14, segments, theme, 3);
    return;
  }
  drawThresholdMeter(ctx, x, middleY, barWidth, 6, meterFraction(row.bar), row.bar.ticks ?? [], theme.accent, theme);
}

function drawListRow(ctx: Canvas2D, row: CardListRow, top: number, height: number, width: number, theme: FamilyTheme): void {
  const middleY = top + height / 2;
  const x = drawGutter(ctx, row.label, middleY, theme);
  const room = Math.max(0, width - CARD_PAD_X - x);
  if (row.missing !== undefined) {
    drawMissingMark(ctx, x, middleY, room, theme, { figure: row.label ?? row.name });
    return;
  }
  if (!row.items.length) {
    line(ctx, row.none, x, middleY, room, `400 12.5px ${theme.font}`, theme.muted);
    return;
  }
  const listFont = `400 12.5px ${theme.font}`;
  const shown = fitItems(row.items, room, text => measureCanvasText(ctx, text, listFont));
  line(ctx, shown, x, middleY, room, listFont, theme.text);
}

function fitItems(items: readonly string[], room: number, measure: (text: string) => number): string {
  const all = items.join(', ');
  if (measure(all) <= room) return all;
  for (let n = items.length - 1; n >= 1; n--) {
    const text = `${items.slice(0, n).join(', ')} +${items.length - n} more`;
    if (measure(text) <= room) return text;
  }
  return all;
}

function drawSection(ctx: Canvas2D, row: CardSectionRow, top: number, height: number, width: number, theme: FamilyTheme): void {
  const middleY = top + height / 2 + 2;
  const used = line(ctx, row.label, CARD_PAD_X, middleY, width - CARD_PAD_X * 2, `600 12px ${theme.font}`, theme.text);
  if (row.count !== undefined) line(ctx, String(row.count), CARD_PAD_X + used + 7, middleY, 80, `400 12px ${theme.font}`, theme.muted);
}

function drawHero(ctx: Canvas2D, row: CardHeroRow, top: number, layout: CardLayout, theme: FamilyTheme, artCache?: LaneArtCache): void {
  const tokens = resolveKitTokens(theme);
  const x = CARD_PAD_X;
  const w = layout.width - CARD_PAD_X * 2;
  const h = CARD_HERO_HEIGHT - 12;
  const titleY = top + 23;
  const stateY = top + 45;
  ctx.save();
  ctx.fillStyle = tokens.raised;
  ctx.strokeStyle = tokens.hairlineStrong;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.roundRect(x + 0.5, top + 0.5, w - 1, h - 1, 10);
  ctx.fill();
  ctx.stroke();
  ctx.restore();
  const inner = x + 14;
  const artW = 54;
  const artH = 48;
  const artMid = top + 12 + artH / 2;
  drawMediaSlot(ctx, inner, artMid, artW, artH, row.art, theme, { artCache, radius: 3 });
  const textX = inner + artW + 14;
  let figureW = 0;
  if (row.figure) {
    const formatted = formatValueState(row.figure.value);
    const figureRight = x + w - 14;
    if (formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable') {
      figureW = drawMissingMark(ctx, figureRight - 150, top + 30, 150, theme, { align: 'right', figure: row.figure.caption, notApplicable: formatted.kind === 'notApplicable' }).width;
    } else {
      figureW = line(ctx, formatted.text, figureRight - 120, top + 26, 120, `600 22px ${theme.font}`, theme.text, 'right');
      figureW = Math.max(figureW, line(ctx, row.figure.caption, figureRight - 120, top + 46, 120, `400 11px ${theme.font}`, theme.muted, 'right'));
    }
  }
  const textW = Math.max(0, x + w - 14 - (figureW ? figureW + 16 : 0) - textX);
  line(ctx, cardTitle(row), textX, titleY, textW, `600 15px ${theme.font}`, row.title.kind === 'notRecorded' ? theme.muted : theme.text);
  const mark = drawStatusMark(ctx, textX, stateY, row.state.mark, theme, { label: row.state.label, maxWidth: textW });
  if (row.detail) line(ctx, row.detail, textX + mark.width + 8, stateY, Math.max(0, textW - mark.width - 8), `400 ${theme.fontSize.small}px ${theme.font}`, theme.muted);
  if (row.meter) drawMeter(ctx, row.meter, x + 11, top + 69, w - 22, 6, theme);
}

function drawItem(ctx: Canvas2D, model: CardModel, row: CardItemRow, top: number, height: number, layout: CardLayout, theme: FamilyTheme, artCache?: LaneArtCache): void {
  const middleY = top + height / 2;
  const columns = model.columns;
  ctx.save();
  if (row.muted) ctx.globalAlpha = MUTED_ROW_ALPHA;
  if (columns.date && row.date) drawDateColumn(ctx, layout.dateX, top, height, row.date.hour, theme);
  if (columns.flag && row.flag) drawMediaSlot(ctx, layout.flagX + layout.flagInset, middleY, CARD_FLAG_WIDTH, CARD_FLAG_HEIGHT, row.flag, theme, { artCache, radius: 3 });
  const shift = columns.date && !row.date ? -(DATE_COLUMN_WIDTH + 12) : 0;
  const titleX = layout.titleX + shift;
  const titleWidth = layout.titleWidth - shift;
  if (columns.art && row.art) drawMediaSlot(ctx, layout.artX + shift, middleY, columns.art.width, columns.art.height, row.art, theme, { artCache, radius: 3 });
  const subtitle = layout.compact && row.place ? (row.subtitle ? `${row.subtitle} · ${row.place}` : row.place) : row.subtitle;
  const titleFont = `${columns.titleWeight ?? 500} 13px ${theme.font}`;
  const titleColor = row.title.kind === 'notRecorded' ? theme.muted : theme.text;
  if (subtitle) {
    line(ctx, cardTitle(row), titleX, middleY - 7, titleWidth, titleFont, titleColor);
    const flagShift = row.subtitleFlag ? CARD_FLAG_WIDTH * 0.8 + 6 : 0;
    if (row.subtitleFlag) drawMediaSlot(ctx, titleX, middleY + 9, CARD_FLAG_WIDTH * 0.8, CARD_FLAG_HEIGHT * 0.8, row.subtitleFlag, theme, { artCache, radius: 2 });
    line(ctx, subtitle, titleX + flagShift, middleY + 9, titleWidth - flagShift, `400 11px ${theme.font}`, theme.muted);
  } else {
    line(ctx, cardTitle(row), titleX, middleY, titleWidth, titleFont, titleColor);
  }
  if (row.place && layout.placeWidth > 0) line(ctx, row.place, layout.placeX, middleY, layout.placeWidth, `400 12.5px ${theme.font}`, theme.muted);
  if (row.meter && layout.meterWidth > 0) {
    drawMeter(ctx, row.meter, layout.meterX, middleY, layout.meterWidth, 6, theme);
    if (row.meter.text) line(ctx, row.meter.text, layout.meterTextX, middleY, layout.meterTextWidth, `600 12px ${theme.font}`, theme.text, 'right');
  }
  if (layout.figureWidth > 0) {
    let figureX = layout.figureX;
    let figureW = layout.figureWidth;
    if (row.meter && layout.meterWidth === 0 && columns.meter) {
      const meterW = Math.min(columns.meter, figureW);
      drawMeter(ctx, row.meter, figureX, middleY, meterW, 4, theme);
      figureX += meterW + 10;
      figureW = Math.max(0, figureW - meterW - 10);
    }
    if (row.figure) drawFigure(ctx, row.figure, figureX, middleY, figureW, columns.figureAlign ?? 'left', theme);
  }
  if (row.state && layout.stateWidth > 0) drawStatusMark(ctx, layout.stateX, middleY, row.state.mark, theme, { label: row.state.label, maxWidth: layout.stateWidth });
  ctx.restore();
}

export function drawCards(ctx: Canvas2D, input: DrawCardsInput): void {
  const { model, layout, theme, scrollTop, artCache, inspectedKey, selectedKey, focusedKey } = input;
  ctx.clearRect(0, 0, layout.width, layout.height);
  const width = layout.rowWidth;
  for (const i of visibleCardRows(layout, scrollTop)) {
    const row = model.rows[i]!;
    const top = layout.rowTops[i]! - scrollTop;
    const height = layout.rowHeights[i]!;
    ctx.save();
    ctx.translate(layout.rowLefts[i]!, 0);
    paintRow(i, row, top, height);
    ctx.restore();
  }
  if (layout.totalHeight - scrollTop > layout.height + 1) {
    drawScrollFade(ctx, 0, layout.height - CARD_FADE, layout.width, CARD_FADE, theme, 'bottom');
  }

  function paintRow(i: number, row: CardModel['rows'][number], top: number, height: number): void {
    if (row.kind === 'section') {
      drawSection(ctx, row, top, height, width, theme);
      return;
    }
    if (row.kind === 'meter' || row.kind === 'list') {
      if (inspectedKey === row.key) drawRowHover(ctx, 0, top, width, height, theme);
      if (row.kind === 'meter') drawMeterRow(ctx, row, top, height, width, theme);
      else drawListRow(ctx, row, top, height, width, theme);
      if (focusedKey === row.key) drawRowFocusRing(ctx, 0, top, width, height, theme);
      return;
    }
    if (row.kind === 'hero') {
      drawHero(ctx, row, top, layout, theme, artCache);
      if (focusedKey === row.key) drawRowFocusRing(ctx, CARD_PAD_X - 4, top - 2, width - CARD_PAD_X * 2 + 8, CARD_HERO_HEIGHT - 8, theme);
      return;
    }
    if (selectedKey === row.key) drawRowSelected(ctx, 0, top, width, height, theme);
    else if (inspectedKey === row.key) drawRowHover(ctx, 0, top, width, height, theme);
    drawItem(ctx, model, row, top, height, layout, theme, artCache);
    const next = model.rows[i + 1];
    if (!next || next.kind === 'item') drawRowHairline(ctx, CARD_PAD_X, width - CARD_PAD_X, top + height, theme, next ? 'normal' : 'strong');
    if (focusedKey === row.key) drawRowFocusRing(ctx, 0, top, width, height, theme);
  }
}

export function hitCards(layout: CardLayout, model: CardModel, point: { readonly x: number; readonly y: number }, scrollTop: number): string | null {
  if (point.x < 0 || point.x > layout.width || point.y < 0 || point.y > layout.height) return null;
  const index = cardRowAt(layout, point.x, point.y, scrollTop);
  if (index < 0) return null;
  const row = model.rows[index]!;
  return row.kind === 'section' ? null : row.key;
}

export function visibleCardIcons(model: CardModel, layout: CardLayout, scrollTop: number): string[] {
  const items: ArtUrlInput[] = [];
  for (const i of visibleCardRows(layout, scrollTop)) {
    const row = model.rows[i]!;
    if (row.kind === 'section' || row.kind === 'meter' || row.kind === 'list') continue;
    items.push(row.art);
    if (row.kind === 'item') items.push(row.flag, row.subtitleFlag);
  }
  return collectArtUrls(items);
}

