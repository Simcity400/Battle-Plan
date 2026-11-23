import { LABEL_WEIGHT, VALUE_WEIGHT } from '../../charts/draw.ts';
import { readTokens } from '../../charts/tokens.ts';
import type { AuditRow } from '../../draw/notifications';
import type { FamilyTheme } from '../contract.ts';
import { drawFormattedValue, drawHatchRect, MISSING_BOX_SIZE, type ValueCanvasContext, type ValueCanvasTheme } from '../valueCanvas.ts';
import { formatValueState, NOT_RECORDED_LABEL } from '../valueFormat.ts';
import type { LogCountryLook } from './declaration.ts';
import {
  LOG_MONTH_HEIGHT,
  resolveLogColumns,
  shortDateParts,
  stickyMonthAt,
  visibleItemRange,
  type LogLayout,
  type ResolvedLogColumn,
} from './layout.ts';
import {
  canonicalLogRowAudit,
  deriveLogAuditRows,
  type FlatLogItem,
  type LogBand,
  type LogDensity,
  type LogModel,
  type LogRowSpec,
  type LogTone,
} from './model.ts';

export interface LogTheme extends ValueCanvasTheme {
  readonly surface: string;
  readonly surfaceAlt: string;
  readonly hover: string;
  readonly selected: string;
  readonly accent: string;
  readonly positive: string;
  readonly negative: string;
  readonly warning: string;
  readonly lineSoft: string;
  readonly categories: readonly string[];
}

export function logThemeFromFamily(theme: FamilyTheme): LogTheme {
  const tokens = typeof document !== 'undefined' ? readTokens() : null;
  const rootStyle = typeof document !== 'undefined' ? getComputedStyle(document.documentElement) : null;
  const surfaceAlt = rootStyle?.getPropertyValue('--surface-raised').trim() || theme.surface;
  const hover = rootStyle?.getPropertyValue('--row-hover').trim() || 'rgba(128,128,128,0.12)';
  const selected = rootStyle?.getPropertyValue('--row-selected').trim() || 'rgba(80,140,255,0.18)';
  return {
    surface: theme.surface,
    surfaceAlt,
    hover,
    selected,
    text: theme.text,
    muted: theme.muted,
    line: theme.line,
    lineSoft: theme.lineSoft,
    accent: theme.accent,
    positive: theme.green,
    negative: theme.red,
    warning: theme.amber,
    hatch: theme.hatch,
    hatchEdge: theme.hatchEdge,
    font: theme.font,
    fontSize: theme.fontSize,
    hatchSpacing: theme.hatchSpacing,
    categories: tokens?.categories ?? [theme.accent, theme.green, theme.amber, theme.red],
  };
}

export interface PaintedSlice {
  readonly geomKey: string;
  readonly sticky: string | null;
  readonly stickyH: number;
  readonly keys: readonly string[];
  readonly tops: readonly number[];
  readonly heights: readonly number[];
  readonly sigs: readonly string[];
}

export interface LogPicture {
  readonly measures: Map<string, number>;
  readonly fitted: Map<string, string>;
  signature: string | null;
  lastSlice: PaintedSlice | null;
}

export function createLogPicture(): LogPicture {
  return {
    measures: new Map(),
    fitted: new Map(),
    signature: null,
    lastSlice: null,
  };
}

export interface DrawLogInput {
  readonly model: LogModel;
  readonly layout: LogLayout;
  readonly density: LogDensity;
  readonly showMonths: boolean;
  readonly width: number;
  readonly height: number;
  readonly scrollTop: number;
  readonly theme: LogTheme;
  readonly inspectedKey: string | null;
  readonly focusedKey: string | null;
  readonly selectedKey: string | null;
  readonly icons: ReadonlyMap<string, ImageBitmap | null>;
  readonly dpr: number;
  readonly prevCanvas?: CanvasImageSource | null;
  readonly wantAudit?: boolean;
  readonly newRowCount?: number;
  readonly rowAlphas?: ReadonlyMap<string, number>;
}

export interface DrawLogResult {
  readonly drew: boolean;
  readonly incremental: boolean;
  readonly auditRows: AuditRow[];
  readonly paintedRows: number;
  readonly visitedItems: number;
  readonly visibleStart: number;
  readonly visibleEnd: number;
}

const FLAG_W = 16;
const FLAG_H = 11;
const ICON_SIZE = 20;
const MEASURE_CAP = 4096;

function fontOf(theme: LogTheme, size: number, weight: number): string {
  return `${weight} ${size}px ${theme.font}`;
}

function toneColor(theme: LogTheme, tone: LogTone | undefined): string {
  switch (tone) {
    case 'muted': return theme.muted;
    case 'accent': return theme.accent;
    case 'positive': return theme.positive;
    case 'negative': return theme.negative;
    case 'warning': return theme.warning;
    default: return theme.text;
  }
}

function measureText(ctx: ValueCanvasContext, picture: LogPicture, text: string, font: string): number {
  const key = `${font}\0${text}`;
  const cached = picture.measures.get(key);
  if (cached !== undefined) return cached;
  ctx.font = font;
  const width = ctx.measureText(text).width;
  if (picture.measures.size >= MEASURE_CAP) picture.measures.clear();
  picture.measures.set(key, width);
  return width;
}

function fitText(ctx: ValueCanvasContext, picture: LogPicture, text: string, font: string, maxWidth: number): string {
  if (maxWidth <= 0 || !text) return '';
  if (measureText(ctx, picture, text, font) <= maxWidth) return text;
  const bucket = Math.floor(maxWidth);
  const key = `${font}\0${bucket}\0${text}`;
  const cached = picture.fitted.get(key);
  if (cached !== undefined) return cached;
  const ellipsis = '…';
  const ellipsisWidth = measureText(ctx, picture, ellipsis, font);
  if (ellipsisWidth >= maxWidth) return '';
  let low = 0;
  let high = text.length;
  while (low < high) {
    const mid = (low + high + 1) >> 1;
    if (measureText(ctx, picture, text.slice(0, mid), font) + ellipsisWidth <= maxWidth) low = mid;
    else high = mid - 1;
  }
  const fitted = low > 0 ? `${text.slice(0, low)}${ellipsis}` : '';
  if (picture.fitted.size >= MEASURE_CAP) picture.fitted.clear();
  picture.fitted.set(key, fitted);
  return fitted;
}

function drawFlag(
  ctx: ValueCanvasContext,
  look: LogCountryLook | null | undefined,
  x: number,
  middleY: number,
  theme: LogTheme,
  icons: ReadonlyMap<string, ImageBitmap | null>,
): number {
  const y = Math.round(middleY - FLAG_H / 2);
  const url = look?.flag;
  const bitmap = url ? icons.get(url) : undefined;
  if (bitmap) {
    ctx.drawImage(bitmap, x, y, FLAG_W, FLAG_H);
    ctx.strokeStyle = theme.line;
    ctx.lineWidth = 0.5;
    ctx.strokeRect(x + 0.25, y + 0.25, FLAG_W - 0.5, FLAG_H - 0.5);
  } else {
    drawHatchRect(ctx, x, y, FLAG_W, FLAG_H, 2, theme);
  }
  return FLAG_W;
}

function drawArt(
  ctx: ValueCanvasContext,
  row: LogRowSpec,
  col: ResolvedLogColumn,
  middleY: number,
  theme: LogTheme,
  icons: ReadonlyMap<string, ImageBitmap | null>,
): void {
  const art = row.art;
  if (!art) return;
  const size = Math.min(ICON_SIZE, Math.max(14, Math.floor(col.width)));
  const x = Math.round(col.left + (col.width - size) / 2);
  const y = Math.round(middleY - size / 2);
  if (art.icon) {
    const bitmap = icons.get(art.icon);
    if (bitmap) {
      ctx.drawImage(bitmap, x, y, size, size);
      return;
    }
  }
  if (art.color) {
    ctx.save();
    ctx.fillStyle = art.color;
    ctx.globalAlpha = 0.18;
    ctx.beginPath();
    ctx.arc(x + size / 2, y + size / 2, size / 2, 0, Math.PI * 2);
    ctx.fill();
    ctx.globalAlpha = 1;
    ctx.fillStyle = art.color;
    ctx.beginPath();
    ctx.arc(x + size / 2, y + size / 2, 3.5, 0, Math.PI * 2);
    ctx.fill();
    ctx.restore();
    return;
  }
  drawHatchRect(ctx, x, y, size, size, 3, theme);
}

function drawBand(
  ctx: ValueCanvasContext,
  picture: LogPicture,
  band: LogBand,
  left: number,
  width: number,
  middleY: number,
  align: 'start' | 'end',
  small: boolean,
  theme: LogTheme,
  icons: ReadonlyMap<string, ImageBitmap | null>,
): void {
  const measure = (text: string, font: string) => measureText(ctx, picture, text, font);
  const fit = (text: string, font: string, maxW: number) => fitText(ctx, picture, text, font, maxW);
  const size = small ? theme.fontSize.small : theme.fontSize.body;
  const right = left + width;

  switch (band.kind) {
    case 'text': {
      const formatted = formatValueState(band.text);
      if (formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable' || formatted.kind === 'partial') {
        drawFormattedValue(ctx, formatted, { left, right, middle: middleY, endAligned: align === 'end', muted: band.tone === 'muted' }, theme, measure, fit);
        return;
      }
      const weight = band.weight === 'bold' ? VALUE_WEIGHT : LABEL_WEIGHT;
      const font = fontOf(theme, size, weight);
      const fitted = fit(formatted.text, font, width);
      if (!fitted) return;
      ctx.font = font;
      ctx.fillStyle = toneColor(theme, band.tone);
      ctx.textBaseline = 'middle';
      ctx.textAlign = align === 'end' ? 'right' : 'left';
      ctx.fillText(fitted, align === 'end' ? right : left, middleY);
      ctx.textAlign = 'left';
      return;
    }
    case 'value': {
      const formatted = formatValueState(band.value, band.format);
      drawFormattedValue(
        ctx,
        formatted,
        { left, right, middle: middleY, endAligned: (band.align ?? align) === 'end', muted: band.tone === 'muted' },
        theme,
        measure,
        fit,
      );
      return;
    }
    case 'pill': {
      const formatted = formatValueState(band.label);
      if (formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable') {
        drawFormattedValue(ctx, formatted, { left, right, middle: middleY, endAligned: align === 'end' }, theme, measure, fit);
        return;
      }
      const pillFont = fontOf(theme, theme.fontSize.small, VALUE_WEIGHT);
      const padX = 7;
      const dotExtra = band.liveDot ? 9 : 0;
      const maxText = Math.max(0, width - padX * 2 - dotExtra);
      const text = fit(formatted.text, pillFont, maxText);
      if (!text) return;
      const textW = measure(text, pillFont);
      const pillW = Math.min(width, textW + padX * 2 + dotExtra);
      const pillH = 18;
      const px = align === 'end' ? right - pillW : left;
      const py = Math.round(middleY - pillH / 2);
      const color = toneColor(theme, band.tone);
      ctx.save();
      ctx.fillStyle = color;
      ctx.globalAlpha = band.tone && band.tone !== 'default' && band.tone !== 'muted' ? 0.14 : 0.08;
      ctx.beginPath();
      ctx.roundRect(px, py, pillW, pillH, 9);
      ctx.fill();
      ctx.globalAlpha = 1;
      let tx = px + padX;
      if (band.liveDot) {
        ctx.fillStyle = color;
        ctx.beginPath();
        ctx.arc(tx + 3, middleY, 3, 0, Math.PI * 2);
        ctx.fill();
        tx += dotExtra;
      }
      ctx.font = pillFont;
      ctx.fillStyle = color;
      ctx.textBaseline = 'middle';
      ctx.textAlign = 'left';
      ctx.fillText(text, tx, middleY);
      ctx.restore();
      return;
    }
    case 'flags': {
      let x = left;
      const max = band.max ?? 3;
      const shown = band.looks.slice(0, max);
      for (const look of shown) {
        if (x + FLAG_W > right) break;
        x += drawFlag(ctx, look, x, middleY, theme, icons) + 4;
      }
      if (shown.length === 1 && shown[0]?.name && x + 20 < right) {
        const font = fontOf(theme, size, LABEL_WEIGHT);
        const text = fit(shown[0].name, font, right - x);
        if (text) {
          ctx.font = font;
          ctx.fillStyle = theme.text;
          ctx.textBaseline = 'middle';
          ctx.fillText(text, x, middleY);
        }
      } else if (shown.length === 1 && !shown[0]?.name && x + 20 < right) {
        const font = fontOf(theme, theme.fontSize.small, LABEL_WEIGHT);
        const text = fit(NOT_RECORDED_LABEL, font, right - x);
        if (text) {
          ctx.font = font;
          ctx.fillStyle = theme.muted;
          ctx.textBaseline = 'middle';
          ctx.fillText(text, x, middleY);
        }
      }
      return;
    }
    case 'rich': {
      let x = left;
      ctx.textBaseline = 'middle';
      ctx.textAlign = 'left';
      for (let i = 0; i < band.parts.length && x < right - 4; i++) {
        const part = band.parts[i]!;
        if (part.flag !== undefined) {
          if (x + FLAG_W > right) break;
          x += drawFlag(ctx, part.flag, x, middleY, theme, icons) + 5;
        }
        const formatted = formatValueState(part.text);
        if (formatted.kind === 'notRecorded') {
          if (x + MISSING_BOX_SIZE > right) break;
          drawHatchRect(ctx, x, Math.round(middleY - MISSING_BOX_SIZE / 2), MISSING_BOX_SIZE, MISSING_BOX_SIZE, 2, theme);
          x += MISSING_BOX_SIZE + 4;
          const font = fontOf(theme, theme.fontSize.small, LABEL_WEIGHT);
          const word = fit(NOT_RECORDED_LABEL, font, right - x);
          if (word) {
            ctx.font = font;
            ctx.fillStyle = theme.muted;
            ctx.fillText(word, x, middleY);
            x += measure(word, font) + 4;
          }
          continue;
        }
        const partText = formatted.kind === 'notApplicable' ? formatted.label : formatted.text;
        if (!partText) continue;
        if (formatted.kind === 'partial') {
          if (x + 6 > right) break;
          ctx.fillStyle = theme.warning;
          ctx.beginPath();
          ctx.arc(x + 2.5, middleY, 2.5, 0, Math.PI * 2);
          ctx.fill();
          x += 7;
        }
        const font = fontOf(theme, size, part.weight === 'bold' ? VALUE_WEIGHT : LABEL_WEIGHT);
        const avail = right - x;
        const text = fit(partText, font, avail);
        if (!text) break;
        ctx.font = font;
        ctx.fillStyle = toneColor(theme, part.tone);
        ctx.fillText(text, x, middleY);
        x += measure(text, font) + 3;
      }
      return;
    }
    case 'meter':
    case 'bar-pair':
    case 'chips':
      return;
  }
}

export function collectLogIcons(_model: LogModel, start: number, end: number, items: readonly FlatLogItem[]): string[] {
  const urls = new Set<string>();
  for (let i = start; i < end; i++) {
    const item = items[i];
    if (!item || item.kind !== 'row') continue;
    const row = item.row;
    if (row.art?.icon) urls.add(row.art.icon);
    const scanBand = (band: LogBand | undefined) => {
      if (!band) return;
      if (band.kind === 'flags') {
        for (const look of band.looks) if (look?.flag) urls.add(look.flag);
      } else if (band.kind === 'rich') {
        for (const part of band.parts) if (part.flag?.flag) urls.add(part.flag.flag);
      }
    };
    for (const slot of row.slots) scanBand(slot);
    scanBand(row.subline);
  }
  return [...urls];
}

function itemIconSig(item: FlatLogItem, icons: ReadonlyMap<string, ImageBitmap | null>): string {
  if (item.kind !== 'row') return '';
  let sig = '';
  const row = item.row;
  if (row.art?.icon) sig += icons.get(row.art.icon) ? '1' : '0';
  for (let s = 0; s < row.slots.length; s++) {
    const band = row.slots[s]!;
    if (band.kind === 'flags') {
      for (const look of band.looks) if (look?.flag) sig += icons.get(look.flag) ? '1' : '0';
    } else if (band.kind === 'rich') {
      for (const part of band.parts) if (part.flag?.flag) sig += icons.get(part.flag.flag) ? '1' : '0';
    }
  }
  if (row.subline?.kind === 'rich') {
    for (const part of row.subline.parts) if (part.flag?.flag) sig += icons.get(part.flag.flag) ? '1' : '0';
  }
  return sig;
}

function paintItem(
  ctx: ValueCanvasContext,
  picture: LogPicture,
  item: FlatLogItem,
  y: number,
  width: number,
  twoLine: boolean,
  dateCol: ResolvedLogColumn | undefined,
  artCol: ResolvedLogColumn | undefined,
  contentCols: readonly ResolvedLogColumn[],
  theme: LogTheme,
  inspectedKey: string | null,
  focusedKey: string | null,
  selectedKey: string | null,
  icons: ReadonlyMap<string, ImageBitmap | null>,
  alpha = 1,
): void {
  const h = item.height;
  if (item.kind === 'month') {
    ctx.fillStyle = theme.surfaceAlt;
    ctx.fillRect(0, y, width, h);
    ctx.font = fontOf(theme, theme.fontSize.small, VALUE_WEIGHT);
    ctx.fillStyle = theme.muted;
    ctx.textBaseline = 'middle';
    ctx.textAlign = 'left';
    ctx.fillText(item.label, 10, y + h / 2);
    ctx.strokeStyle = theme.lineSoft;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(0, y + h - 0.5);
    ctx.lineTo(width, y + h - 0.5);
    ctx.stroke();
    return;
  }

  if (item.kind === 'gap') {
    const inspected = inspectedKey === item.key;
    const focused = focusedKey === item.key;
    drawHatchRect(ctx, 4, y + 2, Math.max(0, width - 8), Math.max(0, h - 4), 4, theme);
    if (inspected) {
      ctx.fillStyle = theme.hover;
      ctx.fillRect(0, y, width, h);
      ctx.fillStyle = theme.muted;
      ctx.fillRect(0, y, 2, h);
    }
    if (focused) {
      ctx.strokeStyle = theme.accent;
      ctx.lineWidth = 1.5;
      ctx.strokeRect(1, y + 1, width - 2, h - 2);
    }
    const gapFont = fontOf(theme, theme.fontSize.small, LABEL_WEIGHT);
    const label = fitText(ctx, picture, item.label, gapFont, Math.max(0, width - 24));
    ctx.font = gapFont;
    ctx.fillStyle = theme.muted;
    ctx.textBaseline = 'middle';
    ctx.fillText(label, 12, y + h / 2);
    return;
  }

  const row = item.row;
  const inspected = inspectedKey === row.key;
  const selected = selectedKey === row.key;
  const focused = focusedKey === row.key;
  const prevAlpha = alpha < 1 ? ctx.globalAlpha : 1;
  if (alpha < 1) {
    ctx.fillStyle = theme.surface;
    ctx.fillRect(0, y, width, h);
    ctx.globalAlpha = prevAlpha * alpha;
  }

  if (selected) {
    ctx.fillStyle = theme.selected;
    ctx.fillRect(0, y, width, h);
  } else if (inspected) {
    ctx.fillStyle = theme.hover;
    ctx.fillRect(0, y, width, h);
  }
  if (inspected) {
    ctx.fillStyle = theme.muted;
    ctx.fillRect(0, y, 2, h);
  }
  if (focused) {
    ctx.strokeStyle = theme.accent;
    ctx.lineWidth = 1.5;
    ctx.strokeRect(1, y + 1, width - 2, h - 2);
  }

  ctx.strokeStyle = theme.lineSoft;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(0, y + h - 0.5);
  ctx.lineTo(width, y + h - 0.5);
  ctx.stroke();

  const middleY = y + h / 2;
  if (dateCol) {
    const parts = shortDateParts(row.hour);
    const dayText = row.by ? `by ${parts.day}` : parts.day;
    const dayFont = fontOf(theme, theme.fontSize.small, VALUE_WEIGHT);
    const yearFont = fontOf(theme, Math.max(10, theme.fontSize.small - 1), LABEL_WEIGHT);
    ctx.textBaseline = 'middle';
    ctx.textAlign = 'left';
    if (twoLine) {
      ctx.font = dayFont;
      ctx.fillStyle = theme.text;
      ctx.fillText(fitText(ctx, picture, dayText, dayFont, dateCol.width), dateCol.left, y + h * 0.34);
      ctx.font = yearFont;
      ctx.fillStyle = theme.muted;
      ctx.fillText(parts.year, dateCol.left, y + h * 0.68);
    } else {
      const line = `${dayText} ${parts.year}`;
      ctx.font = dayFont;
      ctx.fillStyle = theme.muted;
      ctx.fillText(fitText(ctx, picture, line, dayFont, dateCol.width), dateCol.left, middleY);
    }
  }

  if (artCol && width >= 360) {
    drawArt(ctx, row, artCol, middleY, theme, icons);
  }

  for (let s = 0; s < contentCols.length && s < row.slots.length; s++) {
    const col = contentCols[s]!;
    const band = row.slots[s]!;
    if (s === 0 && twoLine && row.subline) {
      drawBand(ctx, picture, band, col.left, col.width, y + h * 0.33, col.align, false, theme, icons);
      drawBand(ctx, picture, row.subline, col.left, col.width, y + h * 0.7, col.align, true, theme, icons);
    } else {
      drawBand(ctx, picture, band, col.left, col.width, middleY, col.align, false, theme, icons);
    }
  }
  if (alpha < 1) ctx.globalAlpha = prevAlpha;
}

function paintStickyMonth(ctx: ValueCanvasContext, sticky: string, width: number, stickyH: number, theme: LogTheme): void {
  ctx.fillStyle = theme.surfaceAlt;
  ctx.fillRect(0, 0, width, stickyH);
  ctx.font = fontOf(theme, theme.fontSize.small, VALUE_WEIGHT);
  ctx.fillStyle = theme.muted;
  ctx.textBaseline = 'middle';
  ctx.textAlign = 'left';
  ctx.fillText(sticky, 10, stickyH / 2);
  ctx.strokeStyle = theme.line;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(0, stickyH - 0.5);
  ctx.lineTo(width, stickyH - 0.5);
  ctx.stroke();
}

export function drawLog(ctx: ValueCanvasContext, picture: LogPicture, input: DrawLogInput): DrawLogResult {
  const {
    model,
    layout,
    density,
    showMonths,
    width,
    height,
    scrollTop,
    theme,
    inspectedKey,
    focusedKey,
    selectedKey,
    icons,
    dpr,
    prevCanvas,
    wantAudit = false,
    newRowCount = 0,
    rowAlphas,
  } = input;
  const [start, end] = visibleItemRange(layout.items, scrollTop, height);
  const sticky = showMonths ? stickyMonthAt(layout, scrollTop) : null;
  const stickyH = sticky ? LOG_MONTH_HEIGHT[density] : 0;

  const auditRows = wantAudit
    ? deriveLogAuditRows(model, layout.items, start, end, scrollTop, stickyH, height, newRowCount)
    : [];
  const visKeys: string[] = [];
  const visTops: number[] = [];
  const visHeights: number[] = [];
  const visSigs: string[] = [];

  for (let i = start; i < end; i++) {
    const item = layout.items[i]!;
    const y = Math.round(item.top - scrollTop);
    const h = item.height;
    if (y + h <= stickyH || y >= height) continue;
    visKeys.push(item.key);
    visTops.push(y);
    visHeights.push(h);
    const hl = (inspectedKey === item.key ? 1 : 0) | (focusedKey === item.key ? 2 : 0) | (selectedKey === item.key ? 4 : 0);
    const alphaRaw = item.kind === 'row' && rowAlphas ? (rowAlphas.get(item.row.key) ?? 1) : 1;
    const alphaPart = alphaRaw < 1 ? `#a${Math.round(alphaRaw * 255)}` : '';
    const base = item.kind === 'row'
      ? `${canonicalLogRowAudit(item.row)}#${hl}#${itemIconSig(item, icons)}${alphaPart}`
      : item.kind === 'month'
        ? `m:${item.label}`
        : `g:${item.label}#${hl}`;
    visSigs.push(base);
  }

  const themeKey = `${theme.surface}|${theme.surfaceAlt}|${theme.text}|${theme.muted}|${theme.accent}|${theme.font}|${theme.fontSize.body}`;
  const geomKey = `${width}x${height}@${dpr}|${density}|${showMonths ? 1 : 0}|${model.twoLine ? 1 : 0}|${themeKey}`;
  const sig = `${geomKey}|${sticky ?? ''}|${visKeys.length}|${visTops.join(',')}|${visKeys.join(',')}|${visSigs.join(';')}`;

  if (picture.signature === sig) {
    return { drew: false, incremental: false, auditRows, paintedRows: 0, visitedItems: 0, visibleStart: start, visibleEnd: end };
  }

  const prev = picture.lastSlice;
  let shiftPrefix = -1;
  let shiftPrevAnchor = 0;
  if (prev && prevCanvas && prev.geomKey === geomKey && prev.stickyH === stickyH && prev.keys.length > 0 && visKeys.length > 1) {
    const maxAnchor = Math.min(2, prev.keys.length);
    for (let p = 0; p < maxAnchor && shiftPrefix < 0; p++) {
      const anchorKey = prev.keys[p]!;
      const anchorSig = prev.sigs[p]!;
      const anchorTop = prev.tops[p]!;
      const anchorHeight = prev.heights[p]!;
      if (anchorTop < stickyH) continue;
      for (let k = p + 1; k < visKeys.length; k++) {
        if (visKeys[k] === anchorKey && visSigs[k] === anchorSig && visHeights[k] === anchorHeight) {
          const delta = visTops[k]! - anchorTop;
          if (delta > 0 && visTops[k]! >= stickyH && visTops[k]! < height) {
            let match = true;
            const tailCount = visKeys.length - k;
            if (p + tailCount > prev.keys.length) {
              match = false;
            } else {
              for (let j = 0; j < tailCount; j++) {
                if (
                  visKeys[k + j] !== prev.keys[p + j]
                  || visSigs[k + j] !== prev.sigs[p + j]
                  || visHeights[k + j] !== prev.heights[p + j]
                  || visTops[k + j]! - prev.tops[p + j]! !== delta
                ) {
                  match = false;
                  break;
                }
              }
            }
            if (match) {
              shiftPrefix = k;
              shiftPrevAnchor = p;
            }
          }
          break;
        }
      }
    }
  }

  const columns = resolveLogColumns(model.columns, width);
  const colById = new Map<string, ResolvedLogColumn>();
  for (const col of columns) colById.set(col.id, col);
  const dateCol = colById.get('date');
  const artCol = colById.get('art');
  const contentCols = columns.filter(col => col.id !== 'date' && col.id !== 'art');
  const twoLine = model.twoLine === true && width >= 300;

  let paintedRows = 0;
  let visitedItems = 0;
  let incremental = false;

  ctx.save();
  if (shiftPrefix > 0 && prevCanvas && prev) {
    const copyStartLogical = prev.tops[shiftPrevAnchor]!;
    const dstStartLogical = visTops[shiftPrefix]!;
    const srcY = Math.round(copyStartLogical * dpr);
    const dstY = Math.round(dstStartLogical * dpr);
    const physW = Math.round(width * dpr);
    const physH = Math.round(height * dpr);
    const copyH = physH - dstY;
    if (srcY >= 0 && dstY > srcY && copyH > 0 && srcY + copyH <= physH) {
      incremental = true;
      ctx.setTransform(1, 0, 0, 1, 0, 0);
      ctx.fillStyle = theme.surface;
      ctx.fillRect(0, 0, physW, dstY);
      ctx.drawImage(prevCanvas, 0, srcY, physW, copyH, 0, dstY, physW, copyH);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

      let visIdx = 0;
      for (let i = start; i < end && visIdx < shiftPrefix; i++) {
        const item = layout.items[i]!;
        const y = Math.round(item.top - scrollTop);
        if (y + item.height <= stickyH || y >= height) continue;
        const alpha = item.kind === 'row' && rowAlphas ? (rowAlphas.get(item.row.key) ?? 1) : 1;
        paintItem(ctx, picture, item, y, width, twoLine, dateCol, artCol, contentCols, theme, inspectedKey, focusedKey, selectedKey, icons, alpha);
        visitedItems++;
        if (item.kind === 'row') paintedRows++;
        visIdx++;
      }

      if (sticky && stickyH > 0) {
        paintStickyMonth(ctx, sticky, width, stickyH, theme);
      }
    }
  }

  if (!incremental) {
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);
    ctx.fillStyle = theme.surface;
    ctx.fillRect(0, 0, width, height);

    for (let i = start; i < end; i++) {
      const item = layout.items[i]!;
      const y = Math.round(item.top - scrollTop);
      const h = item.height;
      if (y + h <= stickyH || y >= height) continue;
      const alpha = item.kind === 'row' && rowAlphas ? (rowAlphas.get(item.row.key) ?? 1) : 1;
      paintItem(ctx, picture, item, y, width, twoLine, dateCol, artCol, contentCols, theme, inspectedKey, focusedKey, selectedKey, icons, alpha);
      visitedItems++;
      if (item.kind === 'row') paintedRows++;
    }

    if (sticky && stickyH > 0) {
      paintStickyMonth(ctx, sticky, width, stickyH, theme);
    }
  }

  ctx.restore();
  picture.signature = sig;
  picture.lastSlice = {
    geomKey,
    sticky,
    stickyH,
    keys: visKeys,
    tops: visTops,
    heights: visHeights,
    sigs: visSigs,
  };
  return { drew: true, incremental, auditRows, paintedRows, visitedItems, visibleStart: start, visibleEnd: end };
}

export function hitLogItem(layout: LogLayout, scrollTop: number, y: number, stickyHeight: number): FlatLogItem | null {
  if (y < stickyHeight) return null;
  const logicalY = scrollTop + y;
  const items = layout.items;
  let low = 0;
  let high = items.length;
  while (low < high) {
    const mid = (low + high) >> 1;
    const item = items[mid]!;
    if (item.top + item.height <= logicalY) low = mid + 1;
    else high = mid;
  }
  const hit = items[low];
  if (!hit || logicalY < hit.top || logicalY >= hit.top + hit.height) return null;
  return hit.kind === 'month' ? null : hit;
}
