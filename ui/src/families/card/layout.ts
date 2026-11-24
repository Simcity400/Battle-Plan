import { DATE_COLUMN_WIDTH } from '../../draw/kit.ts';
import type { CardModel, CardRow } from './model.ts';

export const CARD_PAD_X = 12;
export const CARD_PAD_TOP = 4;
export const CARD_PAD_BOTTOM = 8;
export const CARD_HERO_HEIGHT = 112;
export const CARD_SECTION_HEIGHT = 30;
export const CARD_FLAG_COLUMN = 48;
export const CARD_FLAG_WIDTH = 28;
export const CARD_FLAG_HEIGHT = 19;
export const CARD_GAP = 14;
export const CARD_COMPACT_BELOW = 520;
export const CARD_FADE = 24;
const MIN_TITLE = 120;

export interface CardLayout {
  readonly width: number;
  readonly height: number;
  readonly rowTops: readonly number[];
  readonly rowHeights: readonly number[];
  readonly totalHeight: number;
  readonly compact: boolean;
  readonly dateX: number;
  readonly flagX: number;
  readonly artX: number;
  readonly titleX: number;
  readonly titleWidth: number;
  readonly placeX: number;
  readonly placeWidth: number;
  readonly meterX: number;
  readonly meterWidth: number;
  readonly meterTextX: number;
  readonly meterTextWidth: number;
  readonly figureX: number;
  readonly figureWidth: number;
  readonly stateX: number;
  readonly stateWidth: number;
}

function rowHeight(row: CardRow, model: CardModel): number {
  if (row.kind === 'hero') return CARD_HERO_HEIGHT;
  if (row.kind === 'section') return CARD_SECTION_HEIGHT;
  return model.rowHeight;
}

export function layoutCards(model: CardModel, width: number, height: number): CardLayout {
  const rowTops: number[] = [];
  const rowHeights: number[] = [];
  let y = CARD_PAD_TOP;
  for (const row of model.rows) {
    const h = rowHeight(row, model);
    rowTops.push(y);
    rowHeights.push(h);
    y += h;
  }
  const totalHeight = y + CARD_PAD_BOTTOM;
  const columns = model.columns;
  const compact = width < CARD_COMPACT_BELOW;
  let left = CARD_PAD_X;
  const dateX = left;
  if (columns.date) left += DATE_COLUMN_WIDTH + 12;
  const flagX = left;
  if (columns.flag) left += CARD_FLAG_COLUMN;
  const artX = left;
  if (columns.art) left += columns.art.width + CARD_GAP;
  const titleX = left;

  let right = width - CARD_PAD_X;
  const stateWidth = columns.state ? Math.min(columns.state, compact ? 104 : columns.state) : 0;
  const stateX = right - stateWidth;
  if (stateWidth) right = stateX - CARD_GAP;
  const figureWidth = !compact && columns.figure ? columns.figure : 0;
  const figureX = right - figureWidth;
  if (figureWidth) right = figureX - CARD_GAP;
  const meterTextWidth = !compact && columns.meter && columns.meterText ? columns.meterText : 0;
  const meterTextX = right - meterTextWidth;
  if (meterTextWidth) right = meterTextX - 10;
  const meterWidth = !compact && columns.meter && columns.meterText ? Math.max(0, Math.min(columns.meter, right - titleX - MIN_TITLE - CARD_GAP)) : 0;
  const meterX = right - meterWidth;
  if (meterWidth) right = meterX - CARD_GAP;

  let titleWidth = Math.max(0, right - titleX);
  let placeX = right;
  let placeWidth = 0;
  if (!compact && columns.place) {
    titleWidth = Math.min(titleWidth, columns.titleWidth ?? Math.max(MIN_TITLE, Math.round(titleWidth * 0.4)));
    placeX = titleX + titleWidth + CARD_GAP;
    placeWidth = Math.max(0, right - placeX);
  }
  return {
    width,
    height,
    rowTops,
    rowHeights,
    totalHeight,
    compact,
    dateX,
    flagX,
    artX,
    titleX,
    titleWidth,
    placeX,
    placeWidth,
    meterX,
    meterWidth,
    meterTextX,
    meterTextWidth,
    figureX,
    figureWidth,
    stateX,
    stateWidth,
  };
}

export function visibleCardRange(layout: CardLayout, scrollTop: number): { readonly first: number; readonly last: number } {
  const top = scrollTop;
  const bottom = scrollTop + layout.height;
  let first = 0;
  let lo = 0;
  let hi = layout.rowTops.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (layout.rowTops[mid]! + layout.rowHeights[mid]! <= top) lo = mid + 1;
    else hi = mid;
  }
  first = lo;
  let last = first;
  while (last < layout.rowTops.length && layout.rowTops[last]! < bottom) last++;
  return { first, last };
}

export function cardRowAt(layout: CardLayout, y: number, scrollTop: number): number {
  const at = y + scrollTop;
  let lo = 0;
  let hi = layout.rowTops.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (layout.rowTops[mid]! + layout.rowHeights[mid]! <= at) lo = mid + 1;
    else hi = mid;
  }
  return lo < layout.rowTops.length && layout.rowTops[lo]! <= at ? lo : -1;
}
