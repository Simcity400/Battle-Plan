import { LABEL_WEIGHT, VALUE_WEIGHT } from '../../charts/draw.ts';
import { ellipsise } from '../../charts/labels.ts';
import { formatDateRange } from '../../dashboard/calendar.ts';
import {
  drawChildRowTint,
  drawClockSummary,
  drawGapBand,
  drawRowFocusRing,
  drawRowHairline,
  drawRowHover,
  drawRowSelected,
  resolveKitTokens,
  type ClockSummaryData,
} from '../../draw/kit.ts';
import type { LaneArtCache } from '../art/laneCache.ts';
import { hasMeasuredValue, type Value } from '../value.ts';
import { drawHatchRect, MISSING_BOX_SIZE } from '../valueCanvas.ts';
import { formatValueState } from '../valueFormat.ts';
import { drawApprovedCell, TREE_CHEVRON_LANE, treeLevelsOf, treeTextStart, type CellRenderContext } from './cellPainters.ts';
import { LIVE_END_GAP, type FamilyColumnLayout, type FamilyTableLayout, type RowLayoutGeometry } from './layout.ts';
import { computeColumnMaxima, resolveCellAtHour, resolveCellPaintModel, type FlatTableRow, type TableCell, type TableRow } from './model.ts';
import {
  CELL_INSET,
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
  type TableSettings,
  type TableTheme,
} from './settings.ts';

export { computeColumnMaxima, TREE_CHEVRON_LANE };

export const WATCHING_SPEED = 24;
export const FAST_SPEED = 168;
export const WATCHING_GLIDE_MS = 180;
export const FAST_GLIDE_MS = 120;
export const TOP_RANK_HOLD_MS = 1500;
export const WATCHING_RANK_HOLD_MS = 2500;
const HAIRLINE_INSET = 10;

export function glideDurationForSpeed(speed: number, reducedMotion = false): number {
  if (reducedMotion || speed > FAST_SPEED) return 0;
  return speed > WATCHING_SPEED ? FAST_GLIDE_MS : WATCHING_GLIDE_MS;
}

export function rankHoldForSpeed(speed: number): number {
  return speed > FAST_SPEED ? TOP_RANK_HOLD_MS : WATCHING_RANK_HOLD_MS;
}

export interface FamilyTablePicture {
  frame: string | null;
  rowKeys: (string | undefined)[];
  rowRefs: (TableRow | undefined)[];
  rowMarks: (string | undefined)[];
  cells: Map<string, (string | undefined)[]>;
  ranks: (string | undefined)[];
  measures: Map<string, number>;
  fittedTexts: Map<string, string>;
  maximaRows?: readonly FlatTableRow[];
  maximaHour?: number;
  maximaCols?: string;
  maxima?: Map<string, number>;
  stockpileRows?: readonly FlatTableRow[];
  stockpileMaxAbs?: number;
  lastMaximaSig?: string;
  lastIconVersion?: number;
  lastHour?: number;
}

export function createTablePicture(): FamilyTablePicture {
  return {
    frame: null,
    rowKeys: [],
    rowRefs: [],
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
  readonly iconVersion?: number;
  readonly artCache?: LaneArtCache;
  readonly dpr?: number;
  readonly hasGroupHeaders?: boolean;
  readonly dateGrouped?: boolean;
  readonly precomputedMaxima?: Map<string, number>;
  readonly staticRowLooks?: boolean;
  readonly rowHeightMode?: 'standard' | 'media';
  readonly rowGeom?: RowLayoutGeometry | null;
  readonly rowIndex?: ReadonlyMap<string, number>;
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

function rowMarkState(
  item: Extract<FlatTableRow, { kind: 'data' }>,
  inspectedKey: string | null | undefined,
  selectedKey: string | null | undefined,
  focusedKey: string | null | undefined,
  lastChild: boolean,
): string {
  const i = inspectedKey === item.key ? 1 : 0;
  const s = selectedKey === item.key ? 2 : 0;
  const f = focusedKey === item.key ? 4 : 0;
  const d = item.isDayEnd ? 8 : 0;
  const l = item.isLiveEnd ? 16 : 0;
  const c = lastChild ? 32 : 0;
  const sd = item.showDate === false ? 64 : 0;
  return String(i | s | f | d | l | c | sd);
}

function nonDataMark(item: Exclude<FlatTableRow, { kind: 'data' }>): string {
  if (item.kind === 'group-header') return `g:${item.label}`;
  if (item.kind === 'gap') return `gap:${item.firstHour}:${item.lastHour}:${item.reason}`;
  if (item.kind === 'refused') return `ref:${item.reason}`;
  return `load:${item.key}`;
}

export function treeColumnOf(columns: readonly Pick<FamilyColumnLayout, 'cellKind'>[]): number {
  const at = columns.findIndex(col => col.cellKind !== 'kind');
  return at < 0 ? 0 : at;
}

function barSpanOf(column: FamilyColumnLayout): number {
  return Math.max(1, column.width - CELL_INSET * 2);
}

function paintRowBackground(
  ctx: TableCanvasContext,
  theme: TableTheme,
  rowWidth: number,
  y: number,
  rowH: number,
  contentH: number,
  isChild: boolean,
  inspected: boolean,
  selected: boolean,
  focused: boolean,
  hairlineKind: 'normal' | 'strong' | 'band',
  hairlineLeft: number,
): void {
  ctx.fillStyle = theme.surface;
  ctx.fillRect(0, y, rowWidth, rowH);
  if (isChild) drawChildRowTint(ctx, 0, y, rowWidth, contentH, theme);
  const plateX = ROW_MARGIN_X;
  const plateW = Math.max(1, rowWidth - ROW_MARGIN_X * 2);
  if (selected) {
    drawRowSelected(ctx, plateX, y + 1, plateW, Math.max(1, contentH - 2), theme);
  } else if (inspected) {
    drawRowHover(ctx, plateX, y + 1, plateW, Math.max(1, contentH - 2), theme);
  }
  if (focused) {
    drawRowFocusRing(ctx, plateX, y + 1, plateW, Math.max(1, contentH - 2), theme);
  }
  if (hairlineKind === 'band') {
    const bandY = y + contentH + Math.floor((rowH - contentH) / 2);
    drawRowHairline(ctx, HAIRLINE_INSET, rowWidth - HAIRLINE_INSET, bandY, theme, 'band');
  } else if (!selected && !inspected) {
    drawRowHairline(ctx, hairlineLeft, rowWidth - HAIRLINE_INSET, y + contentH - 1, theme, hairlineKind);
  }
}

function paintCellSliceGround(
  ctx: TableCanvasContext,
  theme: TableTheme,
  x: number,
  y: number,
  cellWidth: number,
  rowH: number,
  contentH: number,
  rowWidth: number,
  isChild: boolean,
  hairlineKind: 'normal' | 'strong' | 'band',
  hairlineLeft: number,
): void {
  ctx.fillStyle = theme.surface;
  ctx.fillRect(x, y, cellWidth, rowH);
  if (isChild) drawChildRowTint(ctx, x, y, cellWidth, contentH, theme);
  const lineY = hairlineKind === 'band' ? y + contentH + Math.floor((rowH - contentH) / 2) : y + contentH - 1;
  drawRowHairline(ctx, Math.max(x, hairlineLeft), Math.min(x + cellWidth, rowWidth - HAIRLINE_INSET), lineY, theme, hairlineKind);
}

function isLastChildRow(rows: readonly FlatTableRow[], index: number): boolean {
  const item = rows[index];
  if (!item || item.kind !== 'data' || item.level <= 1) return false;
  const next = rows[index + 1];
  return !next || next.kind !== 'data' || next.level === 1;
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
    iconVersion,
    artCache,
    dpr = 1,
    precomputedMaxima,
    staticRowLooks,
    dateGrouped = false,
    rowHeightMode = 'standard',
    rowGeom = null,
    rowIndex,
    clockSummary,
  } = input;

  if (clockSummary) {
    if (picture) picture.frame = null;
    ctx.clearRect(0, 0, width, height);
    ctx.fillStyle = theme.surface;
    ctx.fillRect(0, 0, width, height);
    drawClockSummary(ctx, width, height, clockSummary, theme);
    return { drew: true, paintedCells: 1, measurements: 0 };
  }

  const uniformRowHeight = rowHeightMode === 'media' ? MEDIA_ROW_HEIGHT[settings.density] : ROW_HEIGHT[settings.density];
  const variableHeights = rowGeom !== null;
  const bounds = scrollBounds(layout, width, scrollLeft);
  const rank = rankShown ? rankWidth : 0;
  const rowWidth = Math.max(width, layout.totalWidth);
  const moving = glideDuration > 0 && glide.size > 0;

  let first = 0;
  let last = rows.length;
  if (!variableHeights) {
    first = Math.max(0, Math.floor(scrollTop / uniformRowHeight));
    last = Math.min(rows.length, Math.ceil((scrollTop + height) / uniformRowHeight) + (moving ? 2 : 0));
  } else {
    let lo = 0;
    let hi = rows.length;
    while (lo < hi) {
      const mid = (lo + hi) >>> 1;
      if (rowGeom.tops[mid]! + rowGeom.heights[mid]! <= scrollTop) lo = mid + 1;
      else hi = mid;
    }
    first = lo;
    let end = first;
    const bottomEdge = scrollTop + height;
    while (end < rows.length && rowGeom.tops[end]! < bottomEdge) end++;
    last = Math.min(rows.length, end + (moving ? 2 : 0));
  }

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

  let maximaCols = '';
  let hasMeasuredCols = false;
  let hasStockpileSum = false;
  for (let c = 0; c < layout.columns.length; c++) {
    const col = layout.columns[c]!;
    if (col.cellKind === 'stockpileSum') hasStockpileSum = true;
    if (col.numeric && measuresColumn(col)) {
      hasMeasuredCols = true;
      maximaCols += `${col.key}:${col.display}|`;
    }
  }
  let maxima: Map<string, number>;
  if (precomputedMaxima) {
    maxima = precomputedMaxima;
    if (picture) {
      picture.maximaRows = rows;
      picture.maximaHour = hour;
      picture.maximaCols = maximaCols;
      picture.maxima = maxima;
    }
  } else if (!hasMeasuredCols) {
    maxima = new Map();
    if (picture) {
      picture.maximaRows = rows;
      picture.maximaHour = hour;
      picture.maximaCols = maximaCols;
      picture.maxima = maxima;
    }
  } else if (picture && picture.maxima && picture.maximaRows === rows && picture.maximaHour === hour && picture.maximaCols === maximaCols) {
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
  let maximaSig = '';
  if (maxima.size > 0) {
    for (const [k, v] of maxima) maximaSig += `${k}=${v};`;
  }

  let stockpileMaxAbs = 0;
  if (hasStockpileSum) {
    if (picture && picture.stockpileRows === rows && picture.stockpileMaxAbs !== undefined) {
      stockpileMaxAbs = picture.stockpileMaxAbs;
    } else {
      for (let i = 0; i < rows.length; i++) {
        const item = rows[i]!;
        if (item.kind !== 'data') continue;
        for (let c = 0; c < layout.columns.length; c++) {
          const col = layout.columns[c]!;
          if (col.cellKind !== 'stockpileSum') continue;
          const b = item.row.cells[col.key]?.stockpile?.balance;
          if (b != null) stockpileMaxAbs = Math.max(stockpileMaxAbs, Math.abs(b));
        }
      }
      if (picture) {
        picture.stockpileRows = rows;
        picture.stockpileMaxAbs = stockpileMaxAbs;
      }
    }
    maximaSig += `__sp=${stockpileMaxAbs};`;
  }

  let frameKey: string | null = null;
  if (picture && !moving && !variableHeights) {
    let colKey = '';
    for (let c = 0; c < layout.columns.length; c++) {
      const col = layout.columns[c]!;
      colKey += `${col.key}:${col.width}:${col.display}:${col.tone ?? ''}:${col.barEligible ? 1 : 0}|`;
    }
    frameKey = `${width}|${height}|${uniformRowHeight}|${rowHeightMode}|${dateGrouped ? 1 : 0}|${settings.barsBehind ?? 'all'}|${settings.sort.column}|${rank}|${scrollTop}|${scrollLeft}|${layout.scrollable ? 1 : 0}|${layout.pinnedCount}|${layout.pinnedWidth}|${first}|${last}|${rows.length ? '' : empty ?? ''}|${colKey}`;
  }

  const intraDayLeftBase = dateGrouped && layout.columns[0]?.cellKind === 'date'
    ? rank + layout.columns[0].width
    : HAIRLINE_INSET;
  const hairlineSpec = (item: Extract<FlatTableRow, { kind: 'data' }>, index: number, pinned: boolean) => {
    if (item.isLiveEnd) return { kind: 'band' as const, left: HAIRLINE_INSET };
    if (item.isDayEnd || item.isTotal || isLastChildRow(rows, index)) return { kind: 'strong' as const, left: HAIRLINE_INSET };
    if (dateGrouped && intraDayLeftBase > HAIRLINE_INSET) {
      const shifted = pinned ? intraDayLeftBase : Math.max(HAIRLINE_INSET, intraDayLeftBase - bounds.left);
      return { kind: 'normal' as const, left: shifted };
    }
    return { kind: 'normal' as const, left: HAIRLINE_INSET };
  };

  const treeLevels = treeLevelsOf(rows);
  const renderCtx: CellRenderContext = {
    ctx,
    theme,
    density: settings.density,
    barsMode: settings.barsBehind ?? 'all',
    sortedColumn: settings.sort.column,
    treeColumn: treeColumnOf(layout.columns),
    treeLevels,
    icons,
    artCache,
    measure,
    fit,
  };

  const drawRankContent = (item: Extract<FlatTableRow, { kind: 'data' }>, x: number, y: number, rowH: number) => {
    const middle = y + rowH / 2;
    const smallFont = fontOf(theme, theme.fontSize.small, LABEL_WEIGHT);
    ctx.font = smallFont;
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    const label = String(item.rank);
    const labelW = measure(label, smallFont);
    ctx.fillStyle = theme.muted;
    ctx.fillText(label, x + CELL_INSET, middle);
    const marker = markers?.get(item.key);
    if (marker && marker.until > now) {
      ctx.fillStyle = marker.delta > 0 ? theme.green : theme.red;
      const markerText = `${marker.delta > 0 ? '\u25b2' : '\u25bc'}${Math.abs(marker.delta)}`;
      ctx.fillText(fit(markerText, smallFont, Math.max(0, rank - CELL_INSET - labelW - 3)), x + CELL_INSET + labelW + 3, middle);
    }
  };

  const rankSignature = (item: FlatTableRow): string => {
    if (item.kind !== 'data') return '';
    const marker = markers?.get(item.key);
    return marker && marker.until > now ? `${item.key}\u0002${item.rank}\u0002${marker.delta}` : `${item.key}\u0002${item.rank}`;
  };

  const artStateOf = (url: string): number => (artCache
    ? (artCache.bitmapOf(url) ? 1 : artCache.stateOfUrl(url) === 'failed' ? 2 : 3)
    : (icons?.get(url) ? 1 : icons?.get(url) === null ? 2 : 0));

  const cellListSignature = (cell: TableCell): string => {
    let sig = '';
    if (cell.flags) {
      for (const flag of cell.flags) sig += `f${flag.tag}\u0005${flag.icon ?? ''}\u0005${flag.icon ? artStateOf(flag.icon) : 0}\u0005${flag.pending ? 1 : 0}${flag.dimmed ? 1 : 0}`;
    }
    if (cell.icons) {
      for (const entry of cell.icons) sig += `i${entry.url ?? ''}\u0005${entry.url ? artStateOf(entry.url) : 0}\u0005${entry.pending ? 1 : 0}\u0005${entry.name}\u0005${entry.count ?? ''}`;
    }
    return sig;
  };

  const computeLook = (item: Extract<FlatTableRow, { kind: 'data' }>, column: FamilyColumnLayout) => {
    const cell = item.row.cells[column.key];
    const paintModel = resolveCellPaintModel(cell, column, hour, maxima, theme);
    const formatted = formatValueState(paintModel.value, paintModel.format, cell?.missingFigure ? { figure: cell.missingFigure } : undefined);
    const span = Math.max(1, barSpanOf(column) * dpr);
    const pixels = paintModel.bar.fraction === null ? -1 : Math.round(paintModel.bar.fraction * span);
    const fraction = paintModel.bar.fraction === null ? null : pixels / span;
    const fill = paintModel.bar.alert ? theme.red : colorOf(theme, paintModel.tone ?? undefined, paintModel.countryColor, fraction);
    const icon = paintModel.icon ?? undefined;
    const iconPending = paintModel.iconPending === true;
    const art = icon === undefined && !iconPending ? '' : `${icon ?? ''}\u0003${icon === undefined ? 3 : artStateOf(icon)}`;
    const treeKey = `${item.isTreeGroup ? (item.isOpen ? 'o' : 'c') : ''}${treeTextStart(item.level, treeLevels)}`;
    const textVal = formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable' ? formatted.label : formatted.text;
    const secVal = cell?.secondaryValue ? (cell.secondaryValue.kind === 'known' || cell.secondaryValue.kind === 'zero' ? String(cell.secondaryValue.value) : cell.secondaryValue.kind) : '';
    const unitArt = cell?.unitIcon ? `${cell.unitIcon}\u0005${artStateOf(cell.unitIcon)}` : (cell?.unitIconPending ? 'p' : '');
    const splitSig = cell?.splitBar ? cell.splitBar.map(s => `${s.fraction}:${s.fill}:${s.hatched ? 1 : 0}`).join(',') : '';
    const extraSig = `${cell?.blank ? 'blank' : ''}${cell?.subtext ?? ''}\u0004${secVal}\u0004${cell?.shortage ? 1 : 0}\u0004${cell?.stockpile ? JSON.stringify(cell.stockpile) : ''}\u0004${cell ? cellListSignature(cell) : ''}\u0004${cell?.statusKind ?? ''}\u0004${cell?.actionLabel ?? ''}\u0004${cell?.inlinePrefix ?? ''}\u0004${cell?.inlineSubtext ?? ''}\u0004${cell?.unitSuffix ?? ''}\u0004${unitArt}\u0004${cell?.boundedDate ?? ''}\u0004${cell?.missingFigure ?? ''}\u0004${splitSig}\u0004${item.showDate === false ? 0 : 1}`;
    const key = `${formatted.kind}\u0002${textVal}\u0002${paintModel.missingName ? 1 : 0}\u0002${pixels}\u0002${fill}\u0002${paintModel.countryColor ?? ''}\u0002${art}\u0002${treeKey}\u0002${item.row.muted ? 1 : 0}\u0002${paintModel.status ?? ''}\u0002${extraSig}`;
    return { paintModel, formatted, fraction, fill, key };
  };

  const drawGroupHeaderRow = (item: Extract<FlatTableRow, { kind: 'group-header' }>, y: number, rowH: number) => {
    ctx.fillStyle = theme.surface;
    ctx.fillRect(0, y, rowWidth, rowH);
    drawRowHairline(ctx, HAIRLINE_INSET, rowWidth - HAIRLINE_INSET, y + rowH - 1, theme, 'normal');
    const smallFont = fontOf(theme, 12, VALUE_WEIGHT);
    ctx.font = smallFont;
    ctx.fillStyle = theme.muted;
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    ctx.fillText(fit(item.label, smallFont, Math.max(1, width - 20)), CELL_INSET, y + rowH / 2);
  };

  const drawGapRow = (item: Extract<FlatTableRow, { kind: 'gap' }>, y: number, rowH: number) => {
    ctx.fillStyle = theme.surface;
    ctx.fillRect(0, y, rowWidth, rowH);
    const bandW = Math.max(1, rowWidth - HAIRLINE_INSET * 2);
    drawGapBand(ctx, HAIRLINE_INSET, y + 3, bandW, Math.max(1, rowH - 6), theme);
    const smallFont = fontOf(theme, 11.5, LABEL_WEIGHT);
    const label = fit(`Not recorded · ${formatDateRange(item.firstHour, item.lastHour)} · ${item.reason}`, smallFont, Math.max(1, width - HAIRLINE_INSET * 2 - 20));
    const labelW = measure(label, smallFont);
    const padX = 7;
    const chipX = HAIRLINE_INSET + 8;
    const chipH = Math.max(14, rowH - 10);
    const chipY = Math.round(y + (rowH - chipH) / 2);
    ctx.save();
    ctx.fillStyle = theme.surface;
    ctx.beginPath();
    ctx.roundRect(chipX - padX, chipY, labelW + padX * 2, chipH, 3);
    ctx.fill();
    ctx.font = smallFont;
    ctx.fillStyle = theme.muted;
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    ctx.fillText(label, chipX, y + rowH / 2);
    ctx.restore();
    drawRowHairline(ctx, HAIRLINE_INSET, rowWidth - HAIRLINE_INSET, y + rowH - 1, theme, 'strong');
  };

  const drawRefusedRow = (item: Extract<FlatTableRow, { kind: 'refused' }>, y: number, rowH: number) => {
    ctx.fillStyle = theme.surface;
    ctx.fillRect(0, y, rowWidth, rowH);
    const middle = y + rowH / 2;
    const boxX = HAIRLINE_INSET + 4;
    drawHatchRect(ctx, boxX, Math.round(middle - MISSING_BOX_SIZE / 2), MISSING_BOX_SIZE, MISSING_BOX_SIZE, 2, theme);
    const smallFont = fontOf(theme, 12, LABEL_WEIGHT);
    ctx.save();
    ctx.font = smallFont;
    ctx.fillStyle = theme.muted;
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    const textX = boxX + MISSING_BOX_SIZE + 8;
    ctx.fillText(fit(item.reason, smallFont, Math.max(1, width - textX - HAIRLINE_INSET)), textX, middle);
    ctx.restore();
    drawRowHairline(ctx, HAIRLINE_INSET, rowWidth - HAIRLINE_INSET, y + rowH - 1, theme, 'strong');
  };

  const drawLoadingRow = (y: number, rowH: number) => {
    const tokens = resolveKitTokens(theme);
    ctx.fillStyle = theme.surface;
    ctx.fillRect(0, y, rowWidth, rowH);
    const barW = Math.min(180, Math.max(48, width - HAIRLINE_INSET * 2 - 40));
    const barH = 10;
    ctx.save();
    ctx.fillStyle = tokens.headerBand;
    ctx.beginPath();
    ctx.roundRect(HAIRLINE_INSET + 4, Math.round(y + (rowH - barH) / 2), barW, barH, 3);
    ctx.fill();
    ctx.restore();
    drawRowHairline(ctx, HAIRLINE_INSET, rowWidth - HAIRLINE_INSET, y + rowH - 1, theme, 'normal');
  };

  const drawNonDataRow = (item: Exclude<FlatTableRow, { kind: 'data' }>, y: number, rowH: number) => {
    if (item.kind === 'group-header') drawGroupHeaderRow(item, y, rowH);
    else if (item.kind === 'gap') drawGapRow(item, y, rowH);
    else if (item.kind === 'refused') drawRefusedRow(item, y, rowH);
    else drawLoadingRow(y, rowH);
  };

  const paintPinnedOverlayForRow = (
    item: Extract<FlatTableRow, { kind: 'data' }>,
    index: number,
    y: number,
    rowH: number,
  ) => {
    const inspected = inspectedKey === item.key;
    const selected = selectedKey === item.key;
    const focused = focusedKey === item.key;
    const isChild = item.level > 1;
    const contentH = item.isLiveEnd ? rowH - LIVE_END_GAP : rowH;
    const hl = hairlineSpec(item, index, true);
    ctx.save();
    ctx.beginPath();
    ctx.rect(0, y, layout.pinnedWidth, rowH);
    ctx.clip();
    paintRowBackground(ctx, theme, rowWidth, y, rowH, contentH, isChild, inspected, selected, focused, hl.kind, hl.left);
    if (rank > 0) drawRankContent(item, 0, y, contentH);
    let px = rank;
    const limit = Math.min(layout.pinnedCount, layout.columns.length);
    for (let c = 0; c < limit; c++) {
      const col = layout.columns[c]!;
      const look = computeLook(item, col);
      drawApprovedCell(renderCtx, {
        item,
        column: col,
        colIndex: c,
        x: px,
        y,
        rowHeight: contentH,
        paintModel: look.paintModel,
        formatted: look.formatted,
        fraction: look.fraction,
        fill: look.fill,
        rowHovered: inspected,
        rowFocused: focused,
        stockpileMaxAbs,
      });
      px += col.width;
    }
    ctx.restore();
  };

  if (picture && frameKey !== null && picture.frame === frameKey) {
    let drew = false;
    let paintedCells = 0;
    const dirtyRows = new Set<number>();
    const fullRepaintRows = new Set<number>();
    const maximaSame = picture.lastMaximaSig === maximaSig;
    const iconsSame = iconVersion !== undefined && picture.lastIconVersion === iconVersion;
    const hourSafe = staticRowLooks === true || picture.lastHour === hour;
    const canFastSkip = maximaSame && iconsSame && hourSafe;

    const skipRow = new Uint8Array(Math.max(0, last - first));
    if (canFastSkip) {
      for (let index = first; index < last; index++) {
        const item = rows[index]!;
        if (item.kind !== 'data') {
          if (picture.rowKeys[index] === item.key && picture.rowMarks[index] === nonDataMark(item)) {
            skipRow[index - first] = 1;
          }
          continue;
        }
        if (picture.rowRefs[index] !== item.row || picture.rowKeys[index] !== item.key) continue;
        const mark = rowMarkState(item, inspectedKey, selectedKey, focusedKey, isLastChildRow(rows, index));
        if (picture.rowMarks[index] !== mark) continue;
        if (rank > 0) {
          const rSig = `${rankSignature(item)}\u0001${mark}`;
          if (picture.ranks[index] !== rSig) continue;
        }
        skipRow[index - first] = 1;
      }
    }

    for (let index = first; index < last; index++) {
      if (skipRow[index - first]) continue;
      const item = rows[index]!;
      if (item.kind !== 'data') {
        const nMark = nonDataMark(item);
        if (picture.rowKeys[index] === item.key && picture.rowMarks[index] === nMark) continue;
        drawNonDataRow(item, index * uniformRowHeight - scrollTop, uniformRowHeight);
        picture.rowKeys[index] = item.key;
        picture.rowRefs[index] = undefined;
        picture.rowMarks[index] = nMark;
        picture.ranks[index] = undefined;
        dirtyRows.add(index);
        drew = true;
        continue;
      }
      const mark = rowMarkState(item, inspectedKey, selectedKey, focusedKey, isLastChildRow(rows, index));
      if (inspectedKey === item.key || selectedKey === item.key || focusedKey === item.key || picture.rowMarks[index] !== mark || picture.rowKeys[index] !== item.key) {
        fullRepaintRows.add(index);
      }
    }

    for (const index of fullRepaintRows) {
      const item = rows[index]!;
      if (item.kind !== 'data') continue;
      const y = index * uniformRowHeight - scrollTop;
      const inspected = inspectedKey === item.key;
      const selected = selectedKey === item.key;
      const focused = focusedKey === item.key;
      const mark = rowMarkState(item, inspectedKey, selectedKey, focusedKey, isLastChildRow(rows, index));
      const hl = hairlineSpec(item, index, false);
      paintRowBackground(ctx, theme, rowWidth, y, uniformRowHeight, uniformRowHeight, item.level > 1, inspected, selected, focused, hl.kind, hl.left);
      let x = rank - bounds.left;
      for (let colIndex = 0; colIndex < layout.columns.length; colIndex++) {
        const column = layout.columns[colIndex]!;
        let drawn = picture.cells.get(column.key);
        if (!drawn) picture.cells.set(column.key, drawn = []);
        const look = computeLook(item, column);
        drawApprovedCell(renderCtx, {
          item,
          column,
          colIndex,
          x,
          y,
          rowHeight: uniformRowHeight,
          paintModel: look.paintModel,
          formatted: look.formatted,
          fraction: look.fraction,
          fill: look.fill,
          rowHovered: inspected,
          rowFocused: focused,
          stockpileMaxAbs,
        });
        drawn[index] = `${look.key}\u0001${mark}`;
        paintedCells++;
        x += column.width;
      }
      if (rank > 0) {
        drawRankContent(item, layout.scrollable ? 0 : -bounds.left, y, uniformRowHeight);
        picture.ranks[index] = `${rankSignature(item)}\u0001${mark}`;
      }
      dirtyRows.add(index);
      drew = true;
    }

    let x = rank - bounds.left;
    for (let colIndex = 0; colIndex < layout.columns.length; colIndex++) {
      const column = layout.columns[colIndex]!;
      let drawn = picture.cells.get(column.key);
      if (!drawn) picture.cells.set(column.key, drawn = []);
      for (let index = first; index < last; index++) {
        if (skipRow[index - first] || fullRepaintRows.has(index)) continue;
        const item = rows[index]!;
        if (item.kind !== 'data') continue;
        const mark = rowMarkState(item, inspectedKey, selectedKey, focusedKey, isLastChildRow(rows, index));
        const look = computeLook(item, column);
        const cellSig = `${look.key}\u0001${mark}`;
        if (drawn[index] === cellSig) continue;
        const y = index * uniformRowHeight - scrollTop;
        const hl = hairlineSpec(item, index, false);
        paintCellSliceGround(ctx, theme, x, y, column.width, uniformRowHeight, uniformRowHeight, rowWidth, item.level > 1, hl.kind, hl.left);
        drawApprovedCell(renderCtx, {
          item,
          column,
          colIndex,
          x,
          y,
          rowHeight: uniformRowHeight,
          paintModel: look.paintModel,
          formatted: look.formatted,
          fraction: look.fraction,
          fill: look.fill,
          rowHovered: false,
          rowFocused: false,
          stockpileMaxAbs,
        });
        drawn[index] = cellSig;
        dirtyRows.add(index);
        paintedCells++;
        drew = true;
      }
      x += column.width;
    }

    if (rank > 0) {
      for (let index = first; index < last; index++) {
        if (skipRow[index - first] || fullRepaintRows.has(index)) continue;
        const item = rows[index]!;
        if (item.kind !== 'data') continue;
        const mark = rowMarkState(item, inspectedKey, selectedKey, focusedKey, isLastChildRow(rows, index));
        const rSig = `${rankSignature(item)}\u0001${mark}`;
        if (picture.ranks[index] === rSig && !dirtyRows.has(index)) continue;
        const rx = layout.scrollable ? 0 : -bounds.left;
        const y = index * uniformRowHeight - scrollTop;
        const hl = hairlineSpec(item, index, false);
        paintCellSliceGround(ctx, theme, rx, y, rank, uniformRowHeight, uniformRowHeight, rowWidth, item.level > 1, hl.kind, hl.left);
        drawRankContent(item, rx, y, uniformRowHeight);
        picture.ranks[index] = rSig;
        dirtyRows.add(index);
        drew = true;
      }
    }

    if (dirtyRows.size > 0 && layout.scrollable && layout.pinnedWidth > 0) {
      for (const index of dirtyRows) {
        const item = rows[index]!;
        if (item.kind !== 'data') continue;
        paintPinnedOverlayForRow(item, index, index * uniformRowHeight - scrollTop, uniformRowHeight);
      }
    }

    for (let index = first; index < last; index++) {
      if (skipRow[index - first]) continue;
      const item = rows[index]!;
      picture.rowKeys[index] = item.key;
      if (item.kind === 'data') {
        picture.rowRefs[index] = item.row;
        picture.rowMarks[index] = rowMarkState(item, inspectedKey, selectedKey, focusedKey, isLastChildRow(rows, index));
      } else {
        picture.rowRefs[index] = undefined;
        picture.rowMarks[index] = nonDataMark(item);
      }
    }
    picture.lastMaximaSig = maximaSig;
    picture.lastIconVersion = iconVersion;
    picture.lastHour = hour;
    return { drew, paintedCells, measurements };
  }

  if (picture) {
    picture.frame = frameKey;
    picture.rowKeys = [];
    picture.rowRefs = [];
    picture.rowMarks = [];
    picture.cells = new Map();
    picture.ranks = [];
    picture.lastMaximaSig = maximaSig;
    picture.lastIconVersion = iconVersion;
    picture.lastHour = hour;
  }

  ctx.clearRect(0, 0, width, height);
  ctx.fillStyle = theme.surface;
  ctx.fillRect(0, 0, width, height);

  if (!rows.length && empty) {
    const bodyFont = fontOf(theme, theme.fontSize.body, LABEL_WEIGHT);
    ctx.font = bodyFont;
    ctx.fillStyle = theme.muted;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(fit(empty, bodyFont, Math.max(1, width - 32)), width / 2, height / 2);
    return { drew: true, paintedCells: 0, measurements };
  }

  let paintedCells = 0;
  ctx.save();
  ctx.beginPath();
  ctx.rect(0, 0, width, height);
  ctx.clip();

  const topAt = (index: number) => (variableHeights ? rowGeom.tops[index]! : index * uniformRowHeight);
  const heightAt = (index: number) => (variableHeights ? rowGeom.heights[index]! : uniformRowHeight);
  const rowY = (item: FlatTableRow, index: number) =>
    topAt(index) - scrollTop + (moving && item.kind === 'data' ? glideOffset(glide.get(item.key), now, glideDuration) : 0);

  const visibleIndices: number[] = [];
  if (moving) {
    const onScreen = (index: number) => {
      const y = rowY(rows[index]!, index);
      return y + heightAt(index) > 0 && y < height;
    };
    for (const index of glidingIndices(rows, glide, rowIndex, first, last)) {
      if (onScreen(index)) visibleIndices.push(index);
    }
  } else {
    for (let index = first; index < last; index++) visibleIndices.push(index);
  }

  const paintPass = (pinnedOnly: boolean) => {
    const startCol = 0;
    const endCol = pinnedOnly ? Math.min(layout.pinnedCount, layout.columns.length) : layout.columns.length;
    for (let vi = 0; vi < visibleIndices.length; vi++) {
      const index = visibleIndices[vi]!;
      const item = rows[index]!;
      const y = rowY(item, index);
      const rowH = heightAt(index);
      if (item.kind !== 'data') {
        if (!pinnedOnly) {
          drawNonDataRow(item, y, rowH);
          if (picture) {
            picture.rowKeys[index] = item.key;
            picture.rowRefs[index] = undefined;
            picture.rowMarks[index] = nonDataMark(item);
          }
        }
        continue;
      }
      const inspected = inspectedKey === item.key;
      const selected = selectedKey === item.key;
      const focused = focusedKey === item.key;
      const mark = rowMarkState(item, inspectedKey, selectedKey, focusedKey, isLastChildRow(rows, index));
      const contentH = item.isLiveEnd ? rowH - LIVE_END_GAP : rowH;
      const hl = hairlineSpec(item, index, pinnedOnly);
      paintRowBackground(ctx, theme, rowWidth, y, rowH, contentH, item.level > 1, inspected, selected, focused, hl.kind, hl.left);
      if (rank > 0) {
        drawRankContent(item, pinnedOnly ? 0 : -bounds.left, y, contentH);
        if (picture && !pinnedOnly) picture.ranks[index] = `${rankSignature(item)}\u0001${mark}`;
      }
      let x = pinnedOnly ? rank : rank - bounds.left;
      for (let colIndex = startCol; colIndex < endCol; colIndex++) {
        const column = layout.columns[colIndex]!;
        const look = computeLook(item, column);
        drawApprovedCell(renderCtx, {
          item,
          column,
          colIndex,
          x,
          y,
          rowHeight: contentH,
          paintModel: look.paintModel,
          formatted: look.formatted,
          fraction: look.fraction,
          fill: look.fill,
          rowHovered: inspected,
          rowFocused: focused,
          stockpileMaxAbs,
        });
        if (!pinnedOnly) {
          paintedCells++;
          if (picture) {
            let drawn = picture.cells.get(column.key);
            if (!drawn) picture.cells.set(column.key, drawn = []);
            drawn[index] = `${look.key}\u0001${mark}`;
          }
        }
        x += column.width;
      }
      if (picture && !pinnedOnly) {
        picture.rowKeys[index] = item.key;
        picture.rowRefs[index] = item.row;
        picture.rowMarks[index] = mark;
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
  }

  ctx.restore();
  return { drew: true, paintedCells, measurements };
}

function glidingIndices(
  rows: readonly FlatTableRow[],
  glide: ReadonlyMap<string, GlideOffset>,
  rowIndex: ReadonlyMap<string, number> | undefined,
  first: number,
  last: number,
): number[] {
  if (!rowIndex) {
    const all: number[] = [];
    for (let index = 0; index < rows.length; index++) all.push(index);
    return all;
  }
  const out: number[] = [];
  for (let index = first; index < last; index++) out.push(index);
  for (const key of glide.keys()) {
    const index = rowIndex.get(key);
    if (index !== undefined && (index < first || index >= last) && rows[index]?.kind === 'data' && rows[index]!.key === key) out.push(index);
  }
  out.sort((a, b) => a - b);
  return out;
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
  rowGeom?: RowLayoutGeometry | null,
  rowIndex?: ReadonlyMap<string, number>,
  viewportHeight?: number,
): { readonly kind: 'row' | 'empty'; readonly rowKey?: string; readonly column?: string; readonly toggle?: boolean; readonly action?: boolean } {
  let item: FlatTableRow | undefined;
  const useGlide = Boolean(glide && glide.size > 0 && glideDuration && glideDuration > 0 && now !== undefined);
  if (useGlide) {
    const candidates = rowIndex && viewportHeight !== undefined && !rowGeom
      ? glidingIndices(rows, glide!, rowIndex, Math.max(0, Math.floor(scrollTop / rowHeight) - 2), Math.min(rows.length, Math.ceil((scrollTop + viewportHeight) / rowHeight) + 2))
      : glidingIndices(rows, glide!, undefined, 0, rows.length);
    for (let c = candidates.length - 1; c >= 0; c--) {
      const i = candidates[c]!;
      const candidate = rows[i]!;
      const top = rowGeom ? rowGeom.tops[i]! : i * rowHeight;
      const h = rowGeom ? rowGeom.heights[i]! : rowHeight;
      const y = top - scrollTop + (candidate.kind === 'data' ? glideOffset(glide!.get(candidate.key), now!, glideDuration) : 0);
      if (point.y >= y && point.y < y + h) {
        item = candidate;
        break;
      }
    }
  } else if (rowGeom) {
    const absY = point.y + scrollTop;
    let lo = 0;
    let hi = rows.length;
    while (lo < hi) {
      const mid = (lo + hi) >>> 1;
      if (rowGeom.tops[mid]! + rowGeom.heights[mid]! <= absY) lo = mid + 1;
      else hi = mid;
    }
    if (lo < rows.length && absY >= rowGeom.tops[lo]! && absY < rowGeom.tops[lo]! + rowGeom.heights[lo]!) {
      item = rows[lo];
    }
  } else {
    const index = Math.floor((point.y + scrollTop) / rowHeight);
    item = rows[index];
  }
  if (!item) return { kind: 'empty' };
  if (item.kind === 'gap' || item.kind === 'refused') return { kind: 'row', rowKey: item.key };
  if (item.kind !== 'data') return { kind: 'empty' };
  const rank = rankShown ? rankWidth : 0;
  const pinned = layout.scrollable && point.x < layout.pinnedWidth;
  const bodyX = pinned ? point.x : point.x + scrollLeft;
  let x = rank;
  for (let colIndex = 0; colIndex < layout.columns.length; colIndex++) {
    const col = layout.columns[colIndex]!;
    if (bodyX >= x && bodyX < x + col.width) {
      const toggle = colIndex === treeColumnOf(layout.columns) && item.isTreeGroup && bodyX - x <= CELL_INSET + (col.leadInset ?? 0) + treeTextStart(item.level, treeLevelsOf(rows)) + 8;
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
      if (cell.unitIcon) urls.add(cell.unitIcon);
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
      if (cell.unitIcon) urls.add(cell.unitIcon);
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
