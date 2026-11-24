import { DATE_COLUMN_WIDTH } from '../../draw/kit.ts';
import type { CardModel, CardRow } from './model.ts';

export const CARD_PAD_X = 12;
export const CARD_PAD_TOP = 4;
export const CARD_PAD_BOTTOM = 8;
export const CARD_HERO_HEIGHT = 94;
export const CARD_SECTION_HEIGHT = 30;
export const CARD_FLAG_COLUMN = 70;
export const CARD_FLAG_INSET = 10;
export const CARD_FLAG_COLUMN_COMPACT = 46;
export const CARD_FLAG_INSET_COMPACT = 6;
export const CARD_FLAG_WIDTH = 20;
export const CARD_FLAG_HEIGHT = 13;
export const CARD_GAP = 14;
export const CARD_COMPACT_BELOW = 520;
export const CARD_FADE = 24;
export const CARD_LIST_GAP = 28;
export const CARD_LIST_MIN = 300;
const MIN_TITLE = 120;

export interface CardLayout {
  readonly width: number;
  readonly height: number;
  readonly rowTops: readonly number[];
  readonly rowHeights: readonly number[];
  readonly rowLefts: readonly number[];
  readonly rowWidth: number;
  readonly listRanges: readonly (readonly [number, number])[];
  readonly totalHeight: number;
  readonly compact: boolean;
  readonly dateX: number;
  readonly flagX: number;
  readonly flagInset: number;
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

function listCount(model: CardModel, width: number): number {
  if (!model.columns.lists) return 1;
  const sections = model.rows.reduce((n, row) => (row.kind === 'section' ? n + 1 : n), 0);
  const fit = Math.floor((width + CARD_LIST_GAP) / (CARD_LIST_MIN + CARD_LIST_GAP));
  return Math.max(1, Math.min(sections, fit));
}

export function layoutCards(model: CardModel, fullWidth: number, height: number): CardLayout {
  const lists = listCount(model, fullWidth);
  const width = lists > 1 ? Math.floor((fullWidth - CARD_LIST_GAP * (lists - 1)) / lists) : fullWidth;
  const rowTops: number[] = [];
  const rowHeights: number[] = [];
  const rowLefts: number[] = [];
  const listRanges: (readonly [number, number])[] = [];
  let y = CARD_PAD_TOP;
  let list = -1;
  let listStart = 0;
  let deepest = 0;
  model.rows.forEach((row, index) => {
    if (lists > 1 && row.kind === 'section' && list < lists - 1) {
      if (list >= 0) listRanges.push([listStart, index]);
      list++;
      listStart = index;
      deepest = Math.max(deepest, y);
      y = CARD_PAD_TOP;
    }
    const h = rowHeight(row, model);
    rowTops.push(y);
    rowHeights.push(h);
    rowLefts.push(lists > 1 ? Math.max(0, list) * (width + CARD_LIST_GAP) : 0);
    y += h;
  });
  listRanges.push([listStart, model.rows.length]);
  const totalHeight = Math.max(deepest, y) + CARD_PAD_BOTTOM;
  const columns = model.columns;
  const compact = width < CARD_COMPACT_BELOW && !!columns.place;
  let left = CARD_PAD_X;
  const dateX = left;
  if (columns.date) left += DATE_COLUMN_WIDTH + 12;
  const flagX = left;
  if (columns.flag) left += compact ? CARD_FLAG_COLUMN_COMPACT : CARD_FLAG_COLUMN;
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
    width: fullWidth,
    height,
    rowTops,
    rowHeights,
    rowLefts,
    rowWidth: width,
    listRanges,
    totalHeight,
    compact,
    dateX,
    flagX,
    flagInset: compact ? CARD_FLAG_INSET_COMPACT : CARD_FLAG_INSET,
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

export function visibleCardRows(layout: CardLayout, scrollTop: number): number[] {
  const out: number[] = [];
  for (const [start, end] of layout.listRanges) {
    let lo = start;
    let hi = end;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (layout.rowTops[mid]! + layout.rowHeights[mid]! <= scrollTop) lo = mid + 1;
      else hi = mid;
    }
    for (let i = lo; i < end && layout.rowTops[i]! < scrollTop + layout.height; i++) out.push(i);
  }
  return out;
}

export function cardRowAt(layout: CardLayout, x: number, y: number, scrollTop: number): number {
  const at = y + scrollTop;
  const range = layout.listRanges.find(([start]) => x >= (layout.rowLefts[start] ?? 0) && x < (layout.rowLefts[start] ?? 0) + layout.rowWidth);
  if (!range) return -1;
  const [start, end] = range;
  let lo = start;
  let hi = end;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (layout.rowTops[mid]! + layout.rowHeights[mid]! <= at) lo = mid + 1;
    else hi = mid;
  }
  return lo < end && layout.rowTops[lo]! <= at ? lo : -1;
}
