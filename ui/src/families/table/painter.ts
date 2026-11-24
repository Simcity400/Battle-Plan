import { LABEL_WEIGHT, VALUE_WEIGHT } from '../../charts/draw.ts';
import { ellipsise } from '../../charts/labels.ts';
import {
  drawChildRowTint,
  drawClockSummary,
  drawRowFocusRing,
  drawRowHairline,
  drawRowHover,
  drawRowSelected,
  drawScrollFade,
  type ClockSummaryData,
} from '../../draw/kit.ts';
import type { LaneArtCache } from '../art/laneCache.ts';
import { hasMeasuredValue, type Value } from '../value.ts';
import { formatValueState } from '../valueFormat.ts';
import { drawApprovedCell, TREE_CHEVRON_LANE, TREE_INDENT_PX, type CellRenderContext } from './cellPainters.ts';
import type { FamilyColumnLayout, FamilyTableLayout } from './layout.ts';
import { computeColumnMaxima, resolveCellAtHour, resolveCellPaintModel, type FlatTableRow } from './model.ts';
import {
  CELL_INSET,
  CHILD_ROW_HEIGHT,
  MEDIA_ROW_HEIGHT,
  ROW_HEIGHT,
  ROW_MARGIN_X,
  glideOffset,
  measuresColumn,
  scrollBounds,
  showsIcon,
  type GlideOffset,
  type RankMarker,
  type TableCanvasContext,
  type TableDensity,
  type TableSettings,
  type TableTheme,
} from './settings.ts';

export { computeColumnMaxima, TREE_CHEVRON_LANE, TREE_INDENT_PX };

export const WATCHING_SPEED = 24;
export const FAST_SPEED = 168;
export const WATCHING_GLIDE_MS = 180;
export const FAST_GLIDE_MS = 120;
export const TOP_RANK_HOLD_MS = 1500;
export const WATCHING_RANK_HOLD_MS = 2500;
const GROUP_HEADER_HEIGHT = 30;

export function glideDurationForSpeed(speed: number, reducedMotion = false): number {
  if (reducedMotion || speed > FAST_SPEED) return 0;
  return speed > WATCHING_SPEED ? FAST_GLIDE_MS : WATCHING_GLIDE_MS;
}

export function rankHoldForSpeed(speed: number): number {
  return speed > FAST_SPEED ? TOP_RANK_HOLD_MS : WATCHING_RANK_HOLD_MS;
}

export function rowHeightOf(
  item: FlatTableRow,
  density: TableDensity,
  mode: 'standard' | 'media' = 'standard',
): number {
  if (item.kind === 'group-header') return GROUP_HEADER_HEIGHT;
  if (item.level > 1) return CHILD_ROW_HEIGHT;
  return mode === 'media' ? MEDIA_ROW_HEIGHT[density] : ROW_HEIGHT[density];
}

export function computeRowLayout(
  rows: readonly FlatTableRow[],
  density: TableDensity,
  mode: 'standard' | 'media' = 'standard',
): { readonly tops: number[]; readonly heights: number[]; readonly totalHeight: number } {
  const tops = new Array<number>(rows.length);
  const heights = new Array<number>(rows.length);
  let y = 0;
  for (let i = 0; i < rows.length; i++) {
    const h = rowHeightOf(rows[i]!, density, mode);
    tops[i] = y;
    heights[i] = h;
    y += h;
  }
  return { tops, heights, totalHeight: y };
}

export interface FamilyTablePicture {
  frame: string | null;
  rowKeys: (string | undefined)[];
  rowMarks: (string | undefined)[];
  cells: Map<string, (string | undefined)[]>;
  ranks: (string | undefined)[];
  measures: Map<string, number>;
  fittedTexts: Map<string, string>;
  maximaRows?: readonly FlatTableRow[];
  maximaHour?: number;
  maximaCols?: string;
  maxima?: Map<string, number>;
}

export function createTablePicture(): FamilyTablePicture {
  return {
    frame: null,
    rowKeys: [],
    rowMarks: [],
    cells: new Map(),
    ranks: [],
    measures: new Map(),
    fittedTexts: new Map(),
  };
}

export interface PaintStats {
  readonly drew: boolean;
  readonly paintedCells: number;
  readonly measurements: number;
}

export interface DrawFamilyTableInput {
  readonly layout: FamilyTableLayout;
  readonly rows: readonly FlatTableRow[];
  readonly settings: TableSettings;
  readonly hour: number;
  readonly width: number;
  readonly height: number;
  readonly scrollLeft: number;
  readonly scrollTop: number;
  readonly rankShown: boolean;
  readonly rankWidth: number;
  readonly theme: TableTheme;
  readonly now: number;
  readonly glideDuration: number;
  readonly glide: ReadonlyMap<string, GlideOffset>;
  readonly markers?: ReadonlyMap<string, RankMarker>;
  readonly inspectedKey?: string | null;
  readonly selectedKey?: string | null;
  readonly focusedKey?: string | null;
  readonly empty?: string;
  readonly icons?: ReadonlyMap<string, ImageBitmap | null>;
  readonly artCache?: LaneArtCache;
  readonly dpr?: number;
  readonly hasGroupHeaders?: boolean;
  readonly rowHeightMode?: 'standard' | 'media';
  readonly clockSummary?: ClockSummaryData;
}

const fontOf = (theme: TableTheme, size: number, weight: number) => `${weight} ${size}px ${theme.font}`;

function colorOf(theme: TableTheme, tone: string | undefined, countryColor: string | null, fraction: number | null): string {
  if (!tone) return theme.text;
  if (tone === 'country') {
    return countryColor ?? theme.text;
  }
  if (tone === 'value-good' || tone === 'value-bad') {
    if (fraction === null) return theme.muted;
    const badness = tone === 'value-bad' ? fraction : 1 - fraction;
    return badness < 1 / 3 ? theme.green : badness < 2 / 3 ? theme.amber : theme.red;
  }
  if (tone in theme) return (theme as unknown as Record<string, string>)[tone]!;
  return theme.text;
}

function rowMarkState(key: string, inspectedKey: string | null | undefined, selectedKey: string | null | undefined, focusedKey: string | null | undefined): string {
  const i = inspectedKey === key ? 1 : 0;
  const s = selectedKey === key ? 2 : 0;
  const f = focusedKey === key ? 4 : 0;
  return String(i | s | f);
}

function computeStockpileMaxAbs(rows: readonly FlatTableRow[], columns: readonly FamilyColumnLayout[]): number {
  const spCol = columns.find(col => col.cellKind === 'stockpileSum');
  if (!spCol) return 0;
  let maxAbs = 0;
  for (const item of rows) {
    if (item.kind !== 'data') continue;
    const bal = item.row.cells[spCol.key]?.stockpile?.balance;
    if (bal !== null && bal !== undefined) {
      const a = Math.abs(bal);
      if (a > maxAbs) maxAbs = a;
    }
  }
  return maxAbs;
}

function isLastChildOfGroup(rows: readonly FlatTableRow[], index: number): boolean {
  const cur = rows[index];
  if (!cur || cur.kind !== 'data' || cur.level <= 1) return false;
  const next = rows[index + 1];
  return !next || next.kind !== 'data' || next.level < cur.level;
}

export function drawFamilyTable(
  ctx: TableCanvasContext,
  input: DrawFamilyTableInput,
  picture?: FamilyTablePicture,
): PaintStats {
  const {
    layout,
    rows,
    settings,
    hour,
    width,
    height,
    scrollLeft,
    scrollTop,
    rankShown,
    rankWidth,
    theme,
    now,
    glideDuration,
    glide,
    markers,
    inspectedKey,
    selectedKey,
    focusedKey,
    empty,
    icons,
    artCache,
    rowHeightMode = 'standard',
    clockSummary,
  } = input;

  if (clockSummary) {
    if (picture) picture.frame = null;
    ctx.clearRect(0, 0, width, height);
    drawClockSummary(ctx, width, height, clockSummary, theme);
    return { drew: true, paintedCells: 1, measurements: 0 };
  }

  const bounds = scrollBounds(layout, width, scrollLeft);
  const rank = rankShown ? rankWidth : 0;
  const rowGeom = computeRowLayout(rows, settings.density, rowHeightMode);
  const moving = glideDuration > 0 && glide.size > 0;

  let measurements = 0;
  const measureCache = picture?.measures ?? new Map<string, number>();
  const fitCache = picture?.fittedTexts ?? new Map<string, string>();

  const measure = (text: string, font: string): number => {
    const key = `${font}\u0000${text}`;
    let w = measureCache.get(key);
    if (w === undefined) {
      if (measureCache.size > 4096) measureCache.clear();
      ctx.font = font;
      w = ctx.measureText(text).width;
      measureCache.set(key, w);
      measurements++;
    }
    return w;
  };

  const fit = (text: string, font: string, maxWidth: number): string => {
    if (maxWidth <= 0) return '';
    if (measure(text, font) <= maxWidth) return text;
    const key = `${font}\u0000${Math.floor(maxWidth)}\u0000${text}`;
    let cached = fitCache.get(key);
    if (cached === undefined) {
      cached = ellipsise(text, maxWidth, sample => measure(sample, font));
      if (fitCache.size > 4096) fitCache.clear();
      fitCache.set(key, cached);
    }
    return cached;
  };

  const maximaCols = JSON.stringify(layout.columns.map(c => (c.numeric && measuresColumn(c) ? [c.key, c.display, c.barEligible ?? null] : null)));
  let maxima: Map<string, number>;
  if (picture && picture.maxima && picture.maximaRows === rows && picture.maximaHour === hour && picture.maximaCols === maximaCols) {
    maxima = picture.maxima;
  } else {
    maxima = computeColumnMaxima(rows, layout.columns, hour);
    if (picture) {
      picture.maximaRows = rows;
      picture.maximaHour = hour;
      picture.maximaCols = maximaCols;
      picture.maxima = maxima;
    }
  }
  const stockpileMaxAbs = computeStockpileMaxAbs(rows, layout.columns);

  if (picture) {
    picture.frame = null;
    picture.rowKeys = [];
    picture.rowMarks = [];
    picture.cells = new Map();
    picture.ranks = [];
  }

  ctx.clearRect(0, 0, width, height);
  ctx.fillStyle = theme.surface;
  ctx.fillRect(0, 0, width, height);

  if (!rows.length && empty) {
    const bodyFont = fontOf(theme, 13, LABEL_WEIGHT);
    ctx.font = bodyFont;
    ctx.fillStyle = theme.muted;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(fit(empty, bodyFont, Math.max(1, width - 32)), width / 2, height / 2);
    return { drew: true, paintedCells: 0, measurements };
  }

  const rowY = (item: FlatTableRow, index: number) =>
    rowGeom.tops[index]! - scrollTop + (glideDuration > 0 && item.kind === 'data' ? glideOffset(glide.get(item.key), now, glideDuration) : 0);

  const visibleIndices: number[] = [];
  for (let index = 0; index < rows.length; index++) {
    const h = rowGeom.heights[index]!;
    const y = rowY(rows[index]!, index);
    if (y + h > -2 && y < height + 2) visibleIndices.push(index);
    else if (!moving && y >= height + 2) break;
  }

  const rc: CellRenderContext = {
    ctx,
    theme,
    density: settings.density,
    barsMode: settings.bars,
    sortedColumn: settings.sort.column,
    icons,
    artCache,
    measure,
    fit,
  };

  const paintRowBackdrop = (index: number, item: FlatTableRow, y: number, rh: number, bandX: number, bandW: number) => {
    ctx.fillStyle = theme.surface;
    ctx.fillRect(bandX, y, bandW, rh);
    if (item.kind === 'group-header') {
      const smallFont = fontOf(theme, 12, 600);
      ctx.font = smallFont;
      ctx.fillStyle = theme.muted;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillText(fit(item.label, smallFont, Math.max(1, width - 20)), CELL_INSET, y + rh / 2);
      drawRowHairline(ctx, bandX + CELL_INSET, bandX + bandW - CELL_INSET, y + rh - 1, theme, 'normal');
      return;
    }
    const isHovered = inspectedKey === item.key;
    const isSelected = selectedKey === item.key;
    const isFocused = focusedKey === item.key;
    if (item.level > 1) {
      drawChildRowTint(ctx, bandX, y, bandW, rh, theme);
    }
    if (isSelected) {
      drawRowSelected(ctx, bandX, y, bandW, rh, theme, ROW_MARGIN_X);
    } else if (isHovered) {
      drawRowHover(ctx, bandX, y, bandW, rh, theme, ROW_MARGIN_X);
    }
    if (isFocused) {
      drawRowFocusRing(ctx, bandX, y, bandW, rh, theme, ROW_MARGIN_X);
    }
    const nextItem = rows[index + 1];
    const nextHighlighted = nextItem?.kind === 'data'
      && (inspectedKey === nextItem.key || selectedKey === nextItem.key || focusedKey === nextItem.key);
    if (!isHovered && !isSelected && !isFocused && !nextHighlighted) {
      const kind = isLastChildOfGroup(rows, index) ? 'strong' : 'normal';
      drawRowHairline(ctx, bandX + CELL_INSET, bandX + bandW - CELL_INSET, y + rh - 1, theme, kind);
    }
  };

  const paintRankCell = (item: Extract<FlatTableRow, { kind: 'data' }>, x: number, y: number, rh: number) => {
    const middle = y + rh / 2;
    const smallFont = fontOf(theme, 12, LABEL_WEIGHT);
    ctx.font = smallFont;
    ctx.textAlign = 'right';
    ctx.textBaseline = 'middle';
    ctx.fillStyle = theme.muted;
    ctx.fillText(String(item.rank), x + rank - 8, middle);
    const marker = markers?.get(item.key);
    if (marker && marker.until > now) {
      ctx.textAlign = 'left';
      ctx.fillStyle = marker.delta > 0 ? theme.green : theme.red;
      const markerText = `${marker.delta > 0 ? '\u25b2' : '\u25bc'}${Math.abs(marker.delta)}`;
      ctx.fillText(fit(markerText, smallFont, Math.max(0, rank - 24)), x + 6, middle);
    }
  };

  let paintedCells = 0;
  ctx.save();
  ctx.beginPath();
  ctx.rect(0, 0, width, height);
  ctx.clip();

  const paintPass = (pinnedOnly: boolean) => {
    const bandW = pinnedOnly ? layout.pinnedWidth : Math.max(width, layout.totalWidth);
    const bandX = pinnedOnly ? 0 : -bounds.left;
    for (let vi = 0; vi < visibleIndices.length; vi++) {
      const index = visibleIndices[vi]!;
      const item = rows[index]!;
      const y = rowY(item, index);
      const rh = rowGeom.heights[index]!;
      paintRowBackdrop(index, item, y, rh, bandX, bandW);
      if (item.kind !== 'data') continue;

      if (rank > 0 && (pinnedOnly || !layout.scrollable)) {
        paintRankCell(item, pinnedOnly ? 0 : -bounds.left, y, rh);
      }

      let x = pinnedOnly ? rank : rank - bounds.left;
      const colLimit = pinnedOnly ? layout.pinnedCount : layout.columns.length;
      for (let colIndex = 0; colIndex < colLimit; colIndex++) {
        const column = layout.columns[colIndex]!;
        if (!pinnedOnly && layout.scrollable && colIndex < layout.pinnedCount) {
          x += column.width;
          continue;
        }
        const cell = item.row.cells[column.key];
        const paintModel = resolveCellPaintModel(cell, column, hour, maxima, theme);
        const formatted = formatValueState(paintModel.value, paintModel.format);
        const fill = paintModel.bar.alert ? theme.red : colorOf(theme, paintModel.tone ?? undefined, paintModel.countryColor, paintModel.bar.fraction);
        drawApprovedCell(rc, {
          item,
          column,
          colIndex,
          x,
          y,
          rowHeight: rh,
          paintModel,
          formatted,
          fraction: paintModel.bar.fraction,
          fill,
          rowHovered: inspectedKey === item.key,
          rowFocused: focusedKey === item.key,
          stockpileMaxAbs,
        });
        if (!pinnedOnly) paintedCells++;
        x += column.width;
      }

      if (picture && !pinnedOnly) {
        picture.rowKeys[index] = item.key;
        picture.rowMarks[index] = rowMarkState(item.key, inspectedKey, selectedKey, focusedKey);
      }
    }
  };

  paintPass(false);

  if (layout.scrollable && layout.pinnedWidth > 0) {
    ctx.save();
    ctx.beginPath();
    ctx.rect(0, 0, layout.pinnedWidth, height);
    ctx.clip();
    paintPass(true);
    ctx.restore();
    if (bounds.left > 0) {
      const shadow = ctx.createLinearGradient(layout.pinnedWidth, 0, layout.pinnedWidth + 12, 0);
      shadow.addColorStop(0, 'rgba(0, 0, 0, 0.35)');
      shadow.addColorStop(1, 'rgba(0, 0, 0, 0)');
      ctx.fillStyle = shadow;
      ctx.fillRect(layout.pinnedWidth, 0, 12, height);
    }
  }

  if (layout.scrollable && bounds.left < bounds.max) {
    drawScrollFade(ctx, Math.max(layout.pinnedWidth, width - 44), 0, Math.min(44, width), height, theme, 'right');
  }
  if (scrollTop + height < rowGeom.totalHeight - 1) {
    drawScrollFade(ctx, 0, Math.max(0, height - 20), width, Math.min(20, height), theme, 'bottom');
  }

  ctx.restore();
  return { drew: true, paintedCells, measurements };
}

export function hitFamilyTable(
  layout: FamilyTableLayout,
  rows: readonly FlatTableRow[],
  point: { readonly x: number; readonly y: number },
  rowHeight: number,
  scrollLeft: number,
  scrollTop: number,
  rankShown: boolean,
  rankWidth: number,
  glide?: ReadonlyMap<string, GlideOffset>,
  now?: number,
  glideDuration?: number,
  density: TableDensity = 'normal',
  rowHeightMode: 'standard' | 'media' = 'standard',
): { readonly kind: 'row' | 'empty'; readonly rowKey?: string; readonly column?: string; readonly toggle?: boolean; readonly action?: boolean } {
  const rowGeom = computeRowLayout(rows, density, rowHeightMode);
  let item: FlatTableRow | undefined;
  const useGlide = Boolean(glide && glide.size > 0 && glideDuration && glideDuration > 0 && now !== undefined);
  for (let i = rows.length - 1; i >= 0; i--) {
    const candidate = rows[i]!;
    const h = rowGeom.heights[i] ?? rowHeight;
    const y = rowGeom.tops[i]! - scrollTop + (useGlide && candidate.kind === 'data' ? glideOffset(glide!.get(candidate.key), now!, glideDuration) : 0);
    if (point.y >= y && point.y < y + h) {
      item = candidate;
      break;
    }
  }
  if (!item || item.kind !== 'data') return { kind: 'empty' };
  const rank = rankShown ? rankWidth : 0;
  const pinned = layout.scrollable && point.x < layout.pinnedWidth;
  const bodyX = pinned ? point.x : point.x + scrollLeft;
  let x = rank;
  for (let colIndex = 0; colIndex < layout.columns.length; colIndex++) {
    const col = layout.columns[colIndex]!;
    if (bodyX >= x && bodyX < x + col.width) {
      const toggle = colIndex === 0 && item.isTreeGroup && bodyX - x <= CELL_INSET + (item.level - 1) * TREE_INDENT_PX + TREE_CHEVRON_LANE + 8;
      const action = col.cellKind === 'action';
      return { kind: 'row', rowKey: item.key, column: col.key, toggle, action };
    }
    x += col.width;
  }
  return { kind: 'row', rowKey: item.key };
}

export function collectVisibleCellIcons(
  rows: readonly FlatTableRow[],
  columns: readonly Pick<FamilyColumnLayout, 'key' | 'display' | 'skillIcon'>[],
  hour: number,
  first: number,
  last: number,
): string[] {
  const urls = new Set<string>();
  const start = Math.max(0, first);
  const end = Math.min(rows.length, last);
  for (let c = 0; c < columns.length; c++) {
    const col = columns[c]!;
    if (col.skillIcon) urls.add(col.skillIcon);
  }
  for (let i = start; i < end; i++) {
    const item = rows[i]!;
    if (item.kind !== 'data') continue;
    for (let c = 0; c < columns.length; c++) {
      const col = columns[c]!;
      const cell = item.row.cells[col.key];
      if (!cell) continue;
      if (cell.flags) {
        for (const fl of cell.flags) if (fl.icon) urls.add(fl.icon);
      }
      if (cell.icons) {
        for (const ic of cell.icons) if (ic.url) urls.add(ic.url);
      }
      if (cell.icon) urls.add(cell.icon);
      if (cell.iconSlot && showsIcon(col.display)) {
        const resolved = resolveCellAtHour(cell, hour);
        if (resolved.icon) urls.add(resolved.icon);
      }
    }
  }
  return [...urls];
}

export function collectCellIcons(rows: readonly FlatTableRow[]): string[] {
  const urls = new Set<string>();
  for (const item of rows) {
    if (item.kind !== 'data') continue;
    for (const key in item.row.cells) {
      const cell = item.row.cells[key]!;
      if (cell.icon) urls.add(cell.icon);
      if (cell.flags) for (const fl of cell.flags) if (fl.icon) urls.add(fl.icon);
      if (cell.icons) for (const ic of cell.icons) if (ic.url) urls.add(ic.url);
      if (cell.looks) for (const look of cell.looks) if (look.flag) urls.add(look.flag);
    }
  }
  return [...urls];
}

export function hasNonZeroMeasured(value: Value): boolean {
  return hasMeasuredValue(value) && value.kind !== 'zero';
}
