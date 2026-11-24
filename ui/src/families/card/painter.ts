import { ellipsise } from '../../charts/labels.ts';
import { textWidth, type Canvas2D } from '../../charts/textWidth.ts';
import {
  DATE_COLUMN_WIDTH,
  drawArtSlot,
  drawDateColumn,
  drawFlagSlot,
  drawMissingMark,
  drawRowFocusRing,
  drawRowHairline,
  drawRowHover,
  drawRowSelected,
  drawScrollFade,
  drawStatusMark,
  drawTrackBar,
  resolveKitTokens,
} from '../../draw/kit.ts';
import { toArtSlotState, toFlagSlotState } from '../art/adapters.ts';
import type { LaneArtCache } from '../art/laneCache.ts';
import type { ArtResult } from '../art/types.ts';
import type { FamilyTheme } from '../contract.ts';
import { formatValueState } from '../valueFormat.ts';
import {
  CARD_FADE,
  CARD_FLAG_HEIGHT,
  CARD_FLAG_WIDTH,
  CARD_HERO_HEIGHT,
  CARD_PAD_X,
  cardRowAt,
  visibleCardRange,
  type CardLayout,
} from './layout.ts';
import { cardTitleText, type CardHeroRow, type CardItemRow, type CardMedia, type CardMeter, type CardModel, type CardSectionRow } from './model.ts';

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
  const shown = ellipsise(text, width, s => textWidth(ctx, s));
  ctx.fillText(shown, align === 'right' ? x + width : x, y);
  return textWidth(ctx, shown);
}

function resolveMedia(media: CardMedia, artCache?: LaneArtCache): ArtResult {
  const art = media.art;
  if (art.status === 'ready' && !art.bitmap && artCache) return artCache.resolve(art) ?? art;
  return art;
}

function drawMedia(ctx: Canvas2D, media: CardMedia | undefined, x: number, middleY: number, width: number, height: number, theme: FamilyTheme, artCache?: LaneArtCache): void {
  if (!media) return;
  const y = Math.round(middleY - height / 2);
  const art = resolveMedia(media, artCache);
  if (media.kind === 'flag') {
    drawFlagSlot(ctx, x, y, toFlagSlotState(art, media.tag), theme, { width, height });
    return;
  }
  drawArtSlot(ctx, x, y, width, height, toArtSlotState(art, media.name), theme, { radius: 3 });
}

function meterFraction(meter: CardMeter): number | null {
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
  drawMedia(ctx, row.art, inner, artMid, artW, artH, theme, artCache);
  const textX = inner + artW + 14;
  let figureW = 0;
  if (row.figure) {
    const formatted = formatValueState(row.figure.value);
    const figureRight = x + w - 14;
    if (formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable') {
      figureW = drawMissingMark(ctx, figureRight - 150, top + 30, 150, theme, { align: 'right', figure: row.figure.caption }).width;
    } else {
      figureW = line(ctx, formatted.text, figureRight - 120, top + 26, 120, `600 22px ${theme.font}`, theme.text, 'right');
      figureW = Math.max(figureW, line(ctx, row.figure.caption, figureRight - 120, top + 46, 120, `400 11px ${theme.font}`, theme.muted, 'right'));
    }
  }
  const textW = Math.max(0, x + w - 14 - (figureW ? figureW + 16 : 0) - textX);
  line(ctx, cardTitleText(row.title), textX, titleY, textW, `600 15px ${theme.font}`, row.title.kind === 'notRecorded' ? theme.muted : theme.text);
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
  if (columns.flag && row.flag) drawMedia(ctx, row.flag, layout.flagX + layout.flagInset, middleY, CARD_FLAG_WIDTH, CARD_FLAG_HEIGHT, theme, artCache);
  const shift = columns.date && !row.date ? -(DATE_COLUMN_WIDTH + 12) : 0;
  const titleX = layout.titleX + shift;
  const titleWidth = layout.titleWidth - shift;
  if (columns.art && row.art) drawMedia(ctx, row.art, layout.artX + shift, middleY, columns.art.width, columns.art.height, theme, artCache);
  const subtitle = layout.compact && row.place ? (row.subtitle ? `${row.subtitle} · ${row.place}` : row.place) : row.subtitle;
  const titleFont = `${columns.titleWeight ?? 500} 13px ${theme.font}`;
  const titleColor = row.title.kind === 'notRecorded' ? theme.muted : theme.text;
  if (subtitle) {
    line(ctx, cardTitleText(row.title), titleX, middleY - 7, titleWidth, titleFont, titleColor);
    line(ctx, subtitle, titleX, middleY + 9, titleWidth, `400 11px ${theme.font}`, theme.muted);
  } else {
    line(ctx, cardTitleText(row.title), titleX, middleY, titleWidth, titleFont, titleColor);
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
    if (row.figure) {
      const formatted = formatValueState(row.figure.value);
      if (formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable') {
        drawMissingMark(ctx, figureX, middleY, figureW, theme, { figure: row.figure.figure, align: columns.figureAlign ?? 'left' });
      } else {
        line(ctx, formatted.text, figureX, middleY, figureW, `400 ${row.figure.muted ? 11 : 12}px ${theme.font}`, row.figure.muted ? theme.muted : theme.text, columns.figureAlign ?? 'left');
      }
    }
  }
  if (row.state && layout.stateWidth > 0) drawStatusMark(ctx, layout.stateX, middleY, row.state.mark, theme, { label: row.state.label, maxWidth: layout.stateWidth });
  ctx.restore();
}

export function drawCards(ctx: Canvas2D, input: DrawCardsInput): void {
  const { model, layout, theme, scrollTop, artCache, inspectedKey, selectedKey, focusedKey } = input;
  ctx.clearRect(0, 0, layout.width, layout.height);
  const { first, last } = visibleCardRange(layout, scrollTop);
  for (let i = first; i < last; i++) {
    const row = model.rows[i]!;
    const top = layout.rowTops[i]! - scrollTop;
    const height = layout.rowHeights[i]!;
    if (row.kind === 'section') {
      drawSection(ctx, row, top, height, layout.width, theme);
      continue;
    }
    if (row.kind === 'hero') {
      drawHero(ctx, row, top, layout, theme, artCache);
      if (focusedKey === row.key) drawRowFocusRing(ctx, CARD_PAD_X - 4, top - 2, layout.width - CARD_PAD_X * 2 + 8, CARD_HERO_HEIGHT - 8, theme);
      continue;
    }
    if (selectedKey === row.key) drawRowSelected(ctx, 0, top, layout.width, height, theme);
    else if (inspectedKey === row.key) drawRowHover(ctx, 0, top, layout.width, height, theme);
    drawItem(ctx, model, row, top, height, layout, theme, artCache);
    const next = model.rows[i + 1];
    if (!next || next.kind === 'item') drawRowHairline(ctx, CARD_PAD_X, layout.width - CARD_PAD_X, top + height, theme, next ? 'normal' : 'strong');
    if (focusedKey === row.key) drawRowFocusRing(ctx, 0, top, layout.width, height, theme);
  }
  if (layout.totalHeight - scrollTop > layout.height + 1) {
    drawScrollFade(ctx, 0, layout.height - CARD_FADE, layout.width, CARD_FADE, theme, 'bottom');
  }
}

export function hitCards(layout: CardLayout, model: CardModel, point: { readonly x: number; readonly y: number }, scrollTop: number): string | null {
  if (point.x < 0 || point.x > layout.width || point.y < 0 || point.y > layout.height) return null;
  const index = cardRowAt(layout, point.y, scrollTop);
  if (index < 0) return null;
  const row = model.rows[index]!;
  return row.kind === 'section' ? null : row.key;
}

export function visibleCardIcons(model: CardModel, layout: CardLayout, scrollTop: number): string[] {
  const { first, last } = visibleCardRange(layout, scrollTop);
  const urls = new Set<string>();
  for (let i = first; i < last; i++) {
    const row = model.rows[i]!;
    if (row.kind === 'section') continue;
    if (row.art?.iconUrl) urls.add(row.art.iconUrl);
    if (row.kind === 'item' && row.flag?.iconUrl) urls.add(row.flag.iconUrl);
  }
  return [...urls];
}

