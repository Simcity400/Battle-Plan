import { LABEL_WEIGHT, VALUE_WEIGHT } from '../../charts/draw.ts';
import { ellipsise } from '../../charts/labels.ts';
import {
  BAR_GAP,
  CELL_INSET,
  DOT_LANE,
  HEADER_CARET_LANE,
  ICON_LANE,
  METER_HEIGHT,
  METER_WIDTH,
  ROW_HEIGHT,
  TRACK_HEIGHT,
  endAligned,
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
import type { LaneArtCache } from '../art/laneCache.ts';
import { hasMeasuredValue, type Value } from '../value.ts';
import { drawFormattedValue, drawHatchRect } from '../valueCanvas.ts';
import { formatValueState } from '../valueFormat.ts';
import type { FamilyColumnLayout, FamilyTableLayout } from './layout.ts';
import { computeColumnMaxima, resolveCellAtHour, resolveCellPaintModel, type FlatTableRow, type TableRow } from './model.ts';

export { computeColumnMaxima };

export const WATCHING_SPEED = 24;
export const FAST_SPEED = 168;
export const WATCHING_GLIDE_MS = 180;
export const FAST_GLIDE_MS = 120;
export const TOP_RANK_HOLD_MS = 1500;
export const WATCHING_RANK_HOLD_MS = 2500;
export const TREE_INDENT_PX = 16;
export const TREE_CHEVRON_LANE = 16;

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
  readonly precomputedMaxima?: Map<string, number>;
  readonly staticRowLooks?: boolean;
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

function paintRowGround(
  ctx: TableCanvasContext,
  theme: TableTheme,
  x: number,
  y: number,
  width: number,
  height: number,
  odd: boolean,
  inspected: boolean,
  selected: boolean,
  divider = true,
): void {
  ctx.fillStyle = theme.surface;
  ctx.fillRect(x, y, width, height);
  if (odd) {
    ctx.globalAlpha = 0.22;
    ctx.fillStyle = theme.lineSoft;
    ctx.fillRect(x, y, width, height);
    ctx.globalAlpha = 1;
  }
  if (selected) {
    ctx.globalAlpha = 0.18;
    ctx.fillStyle = theme.accent;
    ctx.fillRect(x, y, width, height);
    ctx.globalAlpha = 1;
  } else if (inspected) {
    ctx.globalAlpha = 0.1;
    ctx.fillStyle = theme.accent;
    ctx.fillRect(x, y, width, height);
    ctx.globalAlpha = 1;
  }
  if (divider) {
    ctx.globalAlpha = 0.65;
    ctx.fillStyle = theme.line;
    ctx.fillRect(x, y + height - 1, width, 1);
    ctx.globalAlpha = 1;
  }
}

function paintRowEdges(
  ctx: TableCanvasContext,
  theme: TableTheme,
  x: number,
  y: number,
  width: number,
  height: number,
  inspected: boolean,
  selected: boolean,
  focused: boolean,
): void {
  if (selected) {
    ctx.fillStyle = theme.accent;
    ctx.fillRect(x, y, 2, height - 1);
  } else if (inspected) {
    ctx.fillStyle = theme.muted;
    ctx.fillRect(x, y, 2, height - 1);
  }
  if (focused) {
    ctx.save();
    ctx.strokeStyle = theme.accent;
    ctx.lineWidth = 2;
    ctx.strokeRect(x + 1, y + 1, Math.max(1, width - 2), Math.max(1, height - 3));
    ctx.restore();
  }
}

function barSpanOf(column: FamilyColumnLayout, slot: number): number {
  const span = column.width - CELL_INSET - (column.numeric ? HEADER_CARET_LANE : CELL_INSET);
  return column.display === 'meterBefore' || column.display === 'meterAfter'
    ? METER_WIDTH
    : column.display === 'track'
      ? Math.max(0, span - slot - BAR_GAP)
      : span;
}

export function drawFamilyTable(
  ctx: TableCanvasContext,
  input: DrawFamilyTableInput,
  picture?: FamilyTablePicture,
): PaintStats {
  const { layout, rows, settings, hour, width, height, scrollLeft, scrollTop, rankShown, rankWidth, theme, now, glideDuration, glide, markers,
    inspectedKey, selectedKey, focusedKey, empty, icons, iconVersion, artCache, dpr = 1, precomputedMaxima, staticRowLooks } = input;
  const rowHeight = ROW_HEIGHT[settings.density];
  const bounds = scrollBounds(layout, width, scrollLeft);
  const rank = rankShown ? rankWidth : 0;
  const first = Math.max(0, Math.floor(scrollTop / rowHeight));
  const last = Math.min(rows.length, Math.ceil((scrollTop + height) / rowHeight) + (glide.size && glideDuration > 0 ? 2 : 0));

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

  const effectiveDisplayOf = (column: FamilyColumnLayout): FamilyColumnLayout['display'] => {
    if (!column.numeric || settings.barsBehind === undefined) return column.display;
    if (column.display !== 'faint' && column.display !== 'plain') return column.display;
    if (settings.barsBehind === 'off') return 'plain';
    if (settings.barsBehind === 'all') return 'faint';
    return column.key === settings.sort.column ? 'faint' : 'plain';
  };
  const effectiveColumns = layout.columns.map(column => {
    const display = effectiveDisplayOf(column);
    return display === column.display ? column : { ...column, display };
  });
  let maximaCols = '';
  let hasMeasuredCols = false;
  for (let c = 0; c < effectiveColumns.length; c++) {
    const col = effectiveColumns[c]!;
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
    maxima = computeColumnMaxima(rows, effectiveColumns, hour);
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
  const moving = glideDuration > 0 && glide.size > 0;
  let frameKey: string | null = null;
  if (picture && !moving) {
    let colKey = '';
    for (let c = 0; c < effectiveColumns.length; c++) {
      const col = effectiveColumns[c]!;
      colKey += `${col.key}:${col.width}:${col.display}:${col.tone ?? ''}|`;
    }
    frameKey = `${width}|${height}|${rowHeight}|${rank}|${scrollTop}|${scrollLeft}|${layout.scrollable ? 1 : 0}|${first}|${last}|${rows.length ? '' : empty ?? ''}|${colKey}`;
  }

  const drawSingleCell = (
    item: Extract<FlatTableRow, { kind: 'data' }>,
    column: FamilyColumnLayout,
    colIndex: number,
    x: number,
    y: number,
    slotWidth: number,
    fraction: number | null,
    fill: string,
    iconUrl: string | undefined,
    iconPending: boolean,
    formatted: ReturnType<typeof formatValueState>,
    missingName: boolean,
  ) => {
    const leftBase = x + CELL_INSET;
    const right = x + column.width - (column.numeric ? HEADER_CARET_LANE : CELL_INSET);
    const middle = y + rowHeight / 2;
    let left = leftBase;

    if (colIndex === 0) {
      if (item.level > 1) left += (item.level - 1) * TREE_INDENT_PX;
      if (item.isTreeGroup) {
        ctx.font = fontOf(theme, theme.fontSize.small, LABEL_WEIGHT);
        ctx.fillStyle = theme.muted;
        ctx.textAlign = 'left';
        ctx.textBaseline = 'middle';
        ctx.fillText(item.isOpen ? '\u25be' : '\u25b8', left, middle);
        left += TREE_CHEVRON_LANE;
      }
    }

    const cell = item.row.cells[column.key];
    if (cell?.iconSlot && showsIcon(column.display)) {
      const icon = iconUrl ? (artCache?.bitmapOf(iconUrl) ?? icons?.get(iconUrl)) : undefined;
      const pending = iconPending || (iconUrl !== undefined && artCache?.stateOfUrl(iconUrl) === 'pending');
      const iconHeight = Math.min(14, rowHeight - 8);
      if (icon) {
        const scale = Math.min(20 / icon.width, iconHeight / icon.height);
        const dw = icon.width * scale, dh = icon.height * scale;
        ctx.drawImage(icon, left + (20 - dw) / 2, middle - dh / 2, dw, dh);
      } else if (!pending) {
        drawHatchRect(ctx, left, Math.round(middle - iconHeight / 2), 20, iconHeight, 2, theme);
      }
      if (column.display === 'flag') return;
      left += ICON_LANE;
    }

    if (missingName) {
      ctx.font = fontOf(theme, theme.fontSize.body, LABEL_WEIGHT);
      ctx.fillStyle = theme.muted;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillText(fit('Name not recorded', ctx.font, Math.max(0, right - left)), left, middle);
      return;
    }

    const end = endAligned(column);
    if (formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable') {
      drawFormattedValue(ctx, formatted, { left, right, middle, endAligned: end, muted: item.row.muted }, theme, measure, fit);
      return;
    }

    const drawBar = (barX: number, barWidth: number, barHeight: number) => {
      if (barWidth <= 0 || fraction === null) return;
      const barY = middle - barHeight / 2;
      ctx.fillStyle = theme.lineSoft;
      ctx.beginPath();
      ctx.roundRect(barX, barY, barWidth, barHeight, barHeight / 2);
      ctx.fill();
      if (fraction > 0) {
        ctx.fillStyle = fill;
        ctx.beginPath();
        ctx.roundRect(barX, barY, Math.max(barHeight, barWidth * fraction), barHeight, barHeight / 2);
        ctx.fill();
      }
    };

    if (column.numeric) {
      switch (column.display) {
        case 'faint':
          if (fraction && fraction > 0) {
            const barWidth = Math.max(0, right - left) * fraction;
            const barHeight = Math.min(16, rowHeight - 8);
            ctx.fillStyle = fill;
            ctx.globalAlpha = 0.24;
            ctx.beginPath();
            ctx.roundRect(right - barWidth, middle - barHeight / 2, barWidth, barHeight, 3);
            ctx.fill();
            ctx.globalAlpha = 1;
          }
          drawFormattedValue(ctx, formatted, { left, right: right - 4, middle, endAligned: true, muted: item.row.muted }, theme, measure, fit);
          return;
        case 'track':
          drawBar(left + slotWidth + BAR_GAP, right - left - slotWidth - BAR_GAP, TRACK_HEIGHT);
          drawFormattedValue(ctx, formatted, { left, right: left + slotWidth, middle, endAligned: false, muted: item.row.muted }, theme, measure, fit);
          return;
        case 'meterBefore':
          drawBar(left, METER_WIDTH, METER_HEIGHT);
          drawFormattedValue(ctx, formatted, { left: left + METER_WIDTH + BAR_GAP, right, middle, endAligned: false, muted: item.row.muted }, theme, measure, fit);
          return;
        case 'meterAfter':
          drawBar(right - METER_WIDTH, METER_WIDTH, METER_HEIGHT);
          drawFormattedValue(ctx, formatted, { left, right: right - METER_WIDTH - BAR_GAP, middle, endAligned: true, muted: item.row.muted }, theme, measure, fit);
          return;
        case 'underline': {
          drawFormattedValue(ctx, formatted, { left, right, middle, endAligned: true, muted: item.row.muted }, theme, measure, fit);
          const bodyFont = fontOf(theme, theme.fontSize.body, VALUE_WEIGHT);
          const lineWidth = Math.min(right - left, measure(formatted.text, bodyFont));
          const lineY = middle + 8;
          ctx.fillStyle = theme.lineSoft;
          ctx.fillRect(right - lineWidth, lineY, lineWidth, 2);
          if (fraction && fraction > 0) {
            ctx.fillStyle = fill;
            ctx.fillRect(right - lineWidth, lineY, lineWidth * fraction, 2);
          }
          return;
        }
        default:
          drawFormattedValue(ctx, formatted, { left, right, middle, endAligned: true, muted: item.row.muted }, theme, measure, fit);
          return;
      }
    }

    const status = cell?.status;
    if (status === 'dot') {
      ctx.fillStyle = fill;
      ctx.beginPath();
      ctx.arc(left + 3, middle, 3, 0, Math.PI * 2);
      ctx.fill();
      left += DOT_LANE;
    } else if (status === 'outline' || status === 'fill') {
      const bodyFont = fontOf(theme, theme.fontSize.body, VALUE_WEIGHT);
      const pillWidth = Math.min(right - left, measure(formatted.text, bodyFont) + 16);
      const pillHeight = Math.min(20, rowHeight - 6);
      ctx.beginPath();
      ctx.roundRect(left, middle - pillHeight / 2, pillWidth, pillHeight, pillHeight / 2);
      if (status === 'fill') {
        ctx.fillStyle = fill;
        ctx.globalAlpha = 0.18;
        ctx.fill();
        ctx.globalAlpha = 1;
      }
      ctx.strokeStyle = fill;
      ctx.globalAlpha = 0.7;
      ctx.stroke();
      ctx.globalAlpha = 1;
      ctx.font = bodyFont;
      ctx.fillStyle = theme.text;
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillText(fit(formatted.text, bodyFont, pillWidth - 12), left + pillWidth / 2, middle);
      return;
    }

    drawFormattedValue(ctx, formatted, { left, right, middle, endAligned: false, muted: item.row.muted }, theme, measure, fit);
  };

  const trackSlotWidth = (column: FamilyColumnLayout): number => {
    if (column.display !== 'track') return 0;
    const bodyFont = fontOf(theme, theme.fontSize.body, VALUE_WEIGHT);
    let widest = 0;
    for (let index = first; index < last; index++) {
      const item = rows[index]!;
      if (item.kind !== 'data') continue;
      const cell = item.row.cells[column.key];
      if (!cell) continue;
      const resolved = resolveCellAtHour(cell, hour);
      const formatted = formatValueState(resolved.value, cell.format ?? column.valueFormat);
      if (formatted.kind === 'known' || formatted.kind === 'zero' || formatted.kind === 'partial') {
        widest = Math.max(widest, measure(formatted.text, bodyFont));
      }
    }
    return Math.ceil(widest);
  };

  const drawRank = (item: Extract<FlatTableRow, { kind: 'data' }>, index: number, x: number, y: number) => {
    const inspected = inspectedKey === item.key;
    const selected = selectedKey === item.key;
    paintRowGround(ctx, theme, x, y, rank, rowHeight, index % 2 === 1, inspected, selected, true);
    const middle = y + rowHeight / 2;
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

  const computeLook = (item: Extract<FlatTableRow, { kind: 'data' }>, column: FamilyColumnLayout, slotWidth: number) => {
    const cell = item.row.cells[column.key];
    const paintModel = resolveCellPaintModel(cell, column, hour, maxima, theme);
    const formatted = formatValueState(paintModel.value, paintModel.format);
    const span = Math.max(1, barSpanOf(column, slotWidth) * dpr);
    const pixels = paintModel.bar.fraction === null ? -1 : Math.round(paintModel.bar.fraction * span);
    const fraction = paintModel.bar.fraction === null ? null : pixels / span;
    const fill = paintModel.bar.alert ? theme.red : colorOf(theme, paintModel.tone ?? undefined, paintModel.countryColor, fraction);
    const icon = paintModel.icon ?? undefined;
    const iconPending = paintModel.iconPending === true;
    const iconState = icon === undefined
      ? (iconPending ? 3 : 0)
      : (artCache
        ? (artCache.bitmapOf(icon) ? 1 : artCache.stateOfUrl(icon) === 'failed' ? 2 : 3)
        : (icons?.get(icon) ? 1 : icons?.get(icon) === null ? 2 : 0));
    const art = icon === undefined && !iconPending ? '' : `${icon ?? ''}\u0003${iconState}`;
    const treeKey = item.isTreeGroup ? (item.isOpen ? 'o' : 'c') : String(item.level);
    const textVal = formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable' ? formatted.label : formatted.text;
    const key = `${formatted.kind}\u0002${textVal}\u0002${paintModel.missingName ? 1 : 0}\u0002${pixels}\u0002${fill}\u0002${slotWidth}\u0002${art}\u0002${treeKey}\u0002${item.row.muted ? 1 : 0}\u0002${paintModel.status ?? ''}`;
    return { formatted, fraction, fill, icon, iconPending, missingName: paintModel.missingName, key };
  };

  const drawGroupHeaderRow = (item: Extract<FlatTableRow, { kind: 'group-header' }>, y: number) => {
    const fullW = Math.max(width, layout.totalWidth);
    ctx.fillStyle = theme.surface;
    ctx.fillRect(0, y, fullW, rowHeight);
    ctx.fillStyle = theme.lineSoft;
    ctx.globalAlpha = 0.45;
    ctx.fillRect(0, y, fullW, rowHeight);
    ctx.globalAlpha = 1;
    ctx.fillStyle = theme.line;
    ctx.fillRect(0, y + rowHeight - 1, fullW, 1);
    const smallFont = fontOf(theme, theme.fontSize.small, VALUE_WEIGHT);
    ctx.font = smallFont;
    ctx.fillStyle = theme.muted;
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    ctx.fillText(fit(item.label, smallFont, Math.max(1, width - 16)), CELL_INSET, y + rowHeight / 2);
  };

  if (picture && frameKey !== null && picture.frame === frameKey) {
    let drew = false;
    let paintedCells = 0;
    const dirtyRows = new Set<number>();
    const rowWidth = Math.max(width, layout.totalWidth);
    const maximaSame = picture.lastMaximaSig === maximaSig;
    const iconsSame = iconVersion !== undefined && picture.lastIconVersion === iconVersion;
    const hourSafe = staticRowLooks === true || picture.lastHour === hour;
    const canFastSkip = maximaSame && iconsSame && hourSafe;

    const skipRow = new Uint8Array(Math.max(0, last - first));
    if (canFastSkip) {
      for (let index = first; index < last; index++) {
        const item = rows[index]!;
        if (item.kind === 'group-header') {
          if (picture.rowKeys[index] === item.key && picture.rowMarks[index] === `g:${item.label}`) {
            skipRow[index - first] = 1;
          }
          continue;
        }
        if (picture.rowRefs[index] !== item.row || picture.rowKeys[index] !== item.key) continue;
        const mark = rowMarkState(item.key, inspectedKey, selectedKey, focusedKey);
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
      if (item.kind !== 'group-header') continue;
      const gMark = `g:${item.label}`;
      if (picture.rowKeys[index] === item.key && picture.rowMarks[index] === gMark) continue;
      drawGroupHeaderRow(item, index * rowHeight - scrollTop);
      picture.rowKeys[index] = item.key;
      picture.rowRefs[index] = undefined;
      picture.rowMarks[index] = gMark;
      picture.ranks[index] = undefined;
      dirtyRows.add(index);
      drew = true;
    }

    let x = rank - bounds.left;
    for (let colIndex = 0; colIndex < effectiveColumns.length; colIndex++) {
      const column = effectiveColumns[colIndex]!;
      const slotWidth = trackSlotWidth(column);
      let drawn = picture.cells.get(column.key);
      if (!drawn) picture.cells.set(column.key, drawn = []);
      for (let index = first; index < last; index++) {
        if (skipRow[index - first]) continue;
        const item = rows[index]!;
        if (item.kind !== 'data') continue;
        const mark = rowMarkState(item.key, inspectedKey, selectedKey, focusedKey);
        const look = computeLook(item, column, slotWidth);
        const cellSig = `${look.key}\u0001${mark}`;
        if (picture.rowKeys[index] === item.key && picture.rowMarks[index] === mark && drawn[index] === cellSig) continue;
        const y = index * rowHeight - scrollTop;
        paintRowGround(ctx, theme, x, y, column.width, rowHeight, index % 2 === 1, inspectedKey === item.key, selectedKey === item.key, true);
        drawSingleCell(item, column, colIndex, x, y, slotWidth, look.fraction, look.fill, look.icon, look.iconPending, look.formatted, look.missingName);
        drawn[index] = cellSig;
        dirtyRows.add(index);
        paintedCells++;
        drew = true;
      }
      x += column.width;
    }

    if (rank > 0) {
      for (let index = first; index < last; index++) {
        if (skipRow[index - first]) continue;
        const item = rows[index]!;
        if (item.kind !== 'data') continue;
        const mark = rowMarkState(item.key, inspectedKey, selectedKey, focusedKey);
        const rSig = `${rankSignature(item)}\u0001${mark}`;
        if (picture.ranks[index] === rSig && !dirtyRows.has(index)) continue;
        drawRank(item, index, layout.scrollable ? 0 : -bounds.left, index * rowHeight - scrollTop);
        picture.ranks[index] = rSig;
        dirtyRows.add(index);
        drew = true;
      }
    }

    if (layout.scrollable && dirtyRows.size > 0 && layout.columns.length > 0) {
      const col0 = layout.columns[0]!;
      const pinnedWidth = rank + col0.width;
      const slot0 = trackSlotWidth(col0);
      for (const index of dirtyRows) {
        const item = rows[index]!;
        if (item.kind !== 'data') continue;
        const y = index * rowHeight - scrollTop;
        ctx.save();
        ctx.beginPath();
        ctx.rect(0, y, pinnedWidth, rowHeight);
        ctx.clip();
        paintRowGround(ctx, theme, rank, y, col0.width, rowHeight, index % 2 === 1, inspectedKey === item.key, selectedKey === item.key, true);
        const look0 = computeLook(item, col0, slot0);
        drawSingleCell(item, col0, 0, rank, y, slot0, look0.fraction, look0.fill, look0.icon, look0.iconPending, look0.formatted, look0.missingName);
        if (rank > 0) drawRank(item, index, 0, y);
        ctx.restore();
        const edge = ctx.createLinearGradient(pinnedWidth - 1, 0, pinnedWidth + 12, 0);
        edge.addColorStop(0, theme.surface);
        edge.addColorStop(1, 'transparent');
        ctx.fillStyle = edge;
        ctx.fillRect(pinnedWidth - 1, y, 13, rowHeight);
      }
    }

    for (const index of dirtyRows) {
      const item = rows[index]!;
      if (item.kind !== 'data') continue;
      const y = index * rowHeight - scrollTop;
      paintRowEdges(ctx, theme, 0, y, rowWidth, rowHeight, inspectedKey === item.key, selectedKey === item.key, focusedKey === item.key);
    }

    for (let index = first; index < last; index++) {
      if (skipRow[index - first]) continue;
      const item = rows[index]!;
      picture.rowKeys[index] = item.key;
      if (item.kind === 'data') {
        picture.rowRefs[index] = item.row;
        picture.rowMarks[index] = rowMarkState(item.key, inspectedKey, selectedKey, focusedKey);
      } else {
        picture.rowRefs[index] = undefined;
        picture.rowMarks[index] = `g:${item.label}`;
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

  const rowY = (item: FlatTableRow, index: number) =>
    index * rowHeight - scrollTop + (glideDuration > 0 && item.kind === 'data' ? glideOffset(glide.get(item.key), now, glideDuration) : 0);

  const visibleIndices: number[] = [];
  if (moving) {
    for (let index = 0; index < rows.length; index++) {
      const y = rowY(rows[index]!, index);
      if (y + rowHeight > 0 && y < height) visibleIndices.push(index);
    }
  } else {
    for (let index = first; index < last; index++) visibleIndices.push(index);
  }

  const paintBody = (pinned = false) => {
    let x = pinned ? rank : rank - bounds.left;
    for (let colIndex = 0; colIndex < effectiveColumns.length; colIndex++) {
      const column = effectiveColumns[colIndex]!;
      const isPinned = pinned && colIndex === 0;
      if (isPinned) x = rank;
      const slotWidth = trackSlotWidth(column);
      let drawn = picture?.cells.get(column.key);
      if (picture && !drawn) picture.cells.set(column.key, drawn = []);
      for (let vi = 0; vi < visibleIndices.length; vi++) {
        const index = visibleIndices[vi]!;
        const item = rows[index]!;
        const y = rowY(item, index);
        if (item.kind === 'group-header') {
          if (colIndex === 0) {
            drawGroupHeaderRow(item, y);
            if (picture && !pinned) {
              picture.rowKeys[index] = item.key;
              picture.rowRefs[index] = undefined;
              picture.rowMarks[index] = `g:${item.label}`;
            }
          }
          continue;
        }
        const mark = rowMarkState(item.key, inspectedKey, selectedKey, focusedKey);
        const look = computeLook(item, column, slotWidth);
        if (drawn && !pinned) {
          drawn[index] = `${look.key}\u0001${mark}`;
          picture!.rowKeys[index] = item.key;
          picture!.rowRefs[index] = item.row;
          picture!.rowMarks[index] = mark;
        }
        paintRowGround(ctx, theme, x, y, column.width, rowHeight, index % 2 === 1, inspectedKey === item.key, selectedKey === item.key, true);
        drawSingleCell(item, column, colIndex, x, y, slotWidth, look.fraction, look.fill, look.icon, look.iconPending, look.formatted, look.missingName);
        if (!pinned) paintedCells++;
      }
      if (isPinned) break;
      x += column.width;
    }
  };

  const paintRanks = (x: number) => {
    if (rank <= 0) return;
    for (let vi = 0; vi < visibleIndices.length; vi++) {
      const index = visibleIndices[vi]!;
      const item = rows[index]!;
      if (item.kind !== 'data') continue;
      drawRank(item, index, x, rowY(item, index));
      if (picture) picture.ranks[index] = `${rankSignature(item)}\u0001${rowMarkState(item.key, inspectedKey, selectedKey, focusedKey)}`;
    }
  };

  paintBody(false);
  if (layout.scrollable) {
    const pinnedWidth = rank + (layout.columns[0]?.width ?? 0);
    ctx.save();
    ctx.beginPath();
    ctx.rect(0, 0, pinnedWidth, height);
    ctx.clip();
    ctx.fillStyle = theme.surface;
    ctx.fillRect(0, 0, pinnedWidth, height);
    paintBody(true);
    paintRanks(0);
    ctx.restore();
    const edge = ctx.createLinearGradient(pinnedWidth - 1, 0, pinnedWidth + 12, 0);
    edge.addColorStop(0, theme.surface);
    edge.addColorStop(1, 'transparent');
    ctx.fillStyle = edge;
    ctx.fillRect(pinnedWidth - 1, 0, 13, height);
  } else {
    paintRanks(-bounds.left);
  }

  const rowWidth = Math.max(width, layout.totalWidth);
  for (let vi = 0; vi < visibleIndices.length; vi++) {
    const index = visibleIndices[vi]!;
    const item = rows[index]!;
    if (item.kind !== 'data') continue;
    paintRowEdges(ctx, theme, 0, rowY(item, index), rowWidth, rowHeight, inspectedKey === item.key, selectedKey === item.key, focusedKey === item.key);
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
): { readonly kind: 'row' | 'empty'; readonly rowKey?: string; readonly column?: string; readonly toggle?: boolean } {
  let item: FlatTableRow | undefined;
  if (glide && glide.size > 0 && glideDuration && glideDuration > 0 && now !== undefined) {
    for (let i = rows.length - 1; i >= 0; i--) {
      const candidate = rows[i]!;
      const y = i * rowHeight - scrollTop + (candidate.kind === 'data' ? glideOffset(glide.get(candidate.key), now, glideDuration) : 0);
      if (point.y >= y && point.y < y + rowHeight) {
        item = candidate;
        break;
      }
    }
  } else {
    const index = Math.floor((point.y + scrollTop) / rowHeight);
    item = rows[index];
  }
  if (!item || item.kind !== 'data') return { kind: 'empty' };
  const rank = rankShown ? rankWidth : 0;
  const firstWidth = layout.columns[0]?.width ?? 0;
  const pinned = layout.scrollable && point.x < rank + firstWidth;
  const bodyX = pinned ? point.x : point.x + scrollLeft;
  let x = rank;
  for (let colIndex = 0; colIndex < layout.columns.length; colIndex++) {
    const col = layout.columns[colIndex]!;
    if (bodyX >= x && bodyX < x + col.width) {
      const toggle = colIndex === 0 && item.isTreeGroup && bodyX - x <= CELL_INSET + (item.level - 1) * TREE_INDENT_PX + TREE_CHEVRON_LANE + 6;
      return { kind: 'row', rowKey: item.key, column: col.key, toggle };
    }
    x += col.width;
  }
  return { kind: 'row', rowKey: item.key };
}

export function collectVisibleCellIcons(
  rows: readonly FlatTableRow[],
  columns: readonly Pick<FamilyColumnLayout, 'key' | 'display'>[],
  hour: number,
  first: number,
  last: number,
): string[] {
  const urls = new Set<string>();
  const start = Math.max(0, first);
  const end = Math.min(rows.length, last);
  for (let i = start; i < end; i++) {
    const item = rows[i]!;
    if (item.kind !== 'data') continue;
    for (let c = 0; c < columns.length; c++) {
      const col = columns[c]!;
      if (!showsIcon(col.display)) continue;
      const cell = item.row.cells[col.key];
      if (!cell?.iconSlot) continue;
      const resolved = resolveCellAtHour(cell, hour);
      if (resolved.icon) urls.add(resolved.icon);
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
      if (cell.looks) for (const look of cell.looks) if (look.flag) urls.add(look.flag);
    }
  }
  return [...urls];
}

export function hasNonZeroMeasured(value: Value): boolean {
  return hasMeasuredValue(value) && value.kind !== 'zero';
}
