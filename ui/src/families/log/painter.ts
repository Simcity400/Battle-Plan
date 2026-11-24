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
  readonly preparePrevCanvas?: () => CanvasImageSource | null;
  readonly wantAudit?: boolean;
  readonly newRowCount?: number;
  readonly rowAlphas?: ReadonlyMap<string, number>;
}

export interface DrawLogResult {
  readonly drew: boolean;
  readonly incremental: boolean;
  readonly auditRows: AuditRow[];
  readonly paintedRows: number;
  readonly paintedItems: number;
  readonly scannedItems: number;
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

const GLYPH_PATHS: Readonly<Record<string, readonly { readonly d?: string; readonly cx?: number; readonly cy?: number; readonly r?: number }[]>> = {
  'chart-bars': [{ d: 'M4 3.5v17h17' }, { d: 'M8 16V11M13 16V6M18 16V9' }],
  anchor: [
    { cx: 12, cy: 3.75, r: 2 },
    { d: 'M13.75 5v1.5h2.75v2.75h-2.75v7.93A5 5 0 0 0 16.77 14l2.33-1.25.76 1.25a8 8 0 0 1-15.72 0l.76-1.25 2.33 1.25a5 5 0 0 0 3.02 3.18V9.25H7.5V6.5h2.75V5' },
  ],
  close: [{ d: 'm6.5 6.5 11 11m0-11-11 11' }],
  search: [{ cx: 11, cy: 11, r: 6.5 }, { d: 'm20 20-4.2-4.2' }],
  handshake: [{ d: 'm3 8 4-3 4 2 2-1 4 1 4 5-4 7-3 1-7-5-4-7ZM7 5l-3 7M17 7l4 5M11 7l-3 4 2 2 4-3 5 5M11 16l3 3M14 14l3 3' }],
};

const cachedPath2D = new Map<string, Path2D>();

function path2DOf(d: string): Path2D | null {
  if (typeof Path2D === 'undefined') return null;
  let path = cachedPath2D.get(d);
  if (!path) {
    path = new Path2D(d);
    cachedPath2D.set(d, path);
  }
  return path;
}

function resolveArtColor(color: string | undefined, theme: LogTheme): string | null {
  if (!color) return null;
  if (color.startsWith('cat:')) {
    const idx = Number(color.slice(4));
    if (Number.isInteger(idx) && idx >= 0 && theme.categories.length > 0) {
      return theme.categories[idx % theme.categories.length] ?? theme.accent;
    }
    return theme.accent;
  }
  return color;
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
  const maxBox = art.glyph ? 24 : ICON_SIZE;
  const size = Math.min(maxBox, Math.max(14, Math.floor(col.width)));
  const x = Math.round(col.left + (col.width - size) / 2);
  const y = Math.round(middleY - size / 2);
  if (art.icon) {
    const bitmap = icons.get(art.icon);
    if (bitmap) {
      ctx.drawImage(bitmap, x, y, size, size);
      return;
    }
  }
  const color = resolveArtColor(art.color, theme);
  if (color) {
    ctx.save();
    const baseAlpha = ctx.globalAlpha;
    ctx.fillStyle = color;
    ctx.globalAlpha = baseAlpha * 0.14;
    ctx.beginPath();
    ctx.roundRect(x, y, size, size, 5);
    ctx.fill();
    ctx.globalAlpha = baseAlpha;
    const shapes = art.glyph ? GLYPH_PATHS[art.glyph] : undefined;
    const rawCtx = ctx as unknown as CanvasRenderingContext2D;
    if (shapes && typeof Path2D !== 'undefined' && typeof rawCtx.translate === 'function' && typeof rawCtx.scale === 'function') {
      const iconPx = 14;
      const scale = iconPx / 24;
      rawCtx.translate(x + (size - iconPx) / 2, y + (size - iconPx) / 2);
      rawCtx.scale(scale, scale);
      rawCtx.strokeStyle = color;
      rawCtx.lineWidth = 1.9;
      rawCtx.lineCap = 'round';
      rawCtx.lineJoin = 'round';
      for (const shape of shapes) {
        if (shape.d) {
          const p = path2DOf(shape.d);
          if (p) rawCtx.stroke(p);
        } else if (shape.cx !== undefined && shape.cy !== undefined && shape.r !== undefined) {
          rawCtx.beginPath();
          rawCtx.arc(shape.cx, shape.cy, shape.r, 0, Math.PI * 2);
          rawCtx.stroke();
        }
      }
    } else {
      ctx.fillStyle = color;
      ctx.beginPath();
      ctx.arc(x + size / 2, y + size / 2, 3.5, 0, Math.PI * 2);
      ctx.fill();
    }
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
      if (band.value.kind === 'notApplicable') return;
      const formatted = formatValueState(band.value, band.format);
      const endAligned = (band.align ?? align) === 'end';
      if (formatted.kind === 'known' || formatted.kind === 'zero') {
        const font = fontOf(theme, size, endAligned ? VALUE_WEIGHT : LABEL_WEIGHT);
        const fitted = fit(formatted.text, font, width);
        if (!fitted) return;
        ctx.font = font;
        ctx.fillStyle = toneColor(theme, band.tone);
        ctx.textBaseline = 'middle';
        ctx.textAlign = endAligned ? 'right' : 'left';
        ctx.fillText(fitted, endAligned ? right : left, middleY);
        ctx.textAlign = 'left';
        return;
      }
      drawFormattedValue(
        ctx,
        formatted,
        { left, right, middle: middleY, endAligned, muted: band.tone === 'muted' },
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
      type RichToken =
        | { readonly kind: 'flag'; readonly look: LogCountryLook | null | undefined; readonly width: number }
        | { readonly kind: 'missing'; readonly font: string; readonly width: number }
        | { readonly kind: 'dot'; readonly width: number }
        | { readonly kind: 'text'; readonly text: string; readonly font: string; readonly tone: LogTone | undefined; readonly width: number };
      const tokens: RichToken[] = [];
      let totalWidth = 0;
      const smallFont = fontOf(theme, theme.fontSize.small, LABEL_WEIGHT);
      const baseFont = fontOf(theme, size, LABEL_WEIGHT);
      for (let i = 0; i < band.parts.length; i++) {
        const part = band.parts[i]!;
        if (part.flag !== undefined) {
          const w = FLAG_W + 5;
          tokens.push({ kind: 'flag', look: part.flag, width: w });
          totalWidth += w;
        }
        const formatted = formatValueState(part.text);
        if (formatted.kind === 'notRecorded') {
          const w = MISSING_BOX_SIZE + 4 + measure(NOT_RECORDED_LABEL, smallFont) + 4;
          tokens.push({ kind: 'missing', font: smallFont, width: w });
          totalWidth += w;
          continue;
        }
        const partText = formatted.kind === 'notApplicable' ? formatted.label : formatted.text;
        if (!partText) continue;
        if (formatted.kind === 'partial') {
          tokens.push({ kind: 'dot', width: 7 });
          totalWidth += 7;
        }
        const font = fontOf(theme, size, part.weight === 'bold' ? VALUE_WEIGHT : LABEL_WEIGHT);
        const w = measure(partText, font);
        tokens.push({ kind: 'text', text: partText, font, tone: part.tone, width: w });
        totalWidth += w;
      }

      let x = left;
      ctx.textBaseline = 'middle';
      ctx.textAlign = 'left';

      const drawToken = (tok: RichToken) => {
        if (tok.kind === 'flag') {
          x += drawFlag(ctx, tok.look, x, middleY, theme, icons) + 5;
        } else if (tok.kind === 'missing') {
          drawHatchRect(ctx, x, Math.round(middleY - MISSING_BOX_SIZE / 2), MISSING_BOX_SIZE, MISSING_BOX_SIZE, 2, theme);
          x += MISSING_BOX_SIZE + 4;
          ctx.font = tok.font;
          ctx.fillStyle = theme.muted;
          ctx.fillText(NOT_RECORDED_LABEL, x, middleY);
          x += measure(NOT_RECORDED_LABEL, tok.font) + 4;
        } else if (tok.kind === 'dot') {
          ctx.fillStyle = theme.warning;
          ctx.beginPath();
          ctx.arc(x + 2.5, middleY, 2.5, 0, Math.PI * 2);
          ctx.fill();
          x += 7;
        } else {
          ctx.font = tok.font;
          ctx.fillStyle = toneColor(theme, tok.tone);
          ctx.fillText(tok.text, x, middleY);
          x += tok.width;
        }
      };

      if (totalWidth <= width) {
        for (let i = 0; i < tokens.length; i++) drawToken(tokens[i]!);
        return;
      }

      const ellipsis = '…';
      const ellipsisW = measure(ellipsis, baseFont);
      if (ellipsisW > width) return;
      const limitX = right - ellipsisW;

      for (let i = 0; i < tokens.length; i++) {
        const tok = tokens[i]!;
        if (x + tok.width <= limitX) {
          drawToken(tok);
          continue;
        }
        const avail = limitX - x;
        if (tok.kind === 'text') {
          let low = 0;
          let high = tok.text.length;
          while (low < high) {
            const mid = (low + high + 1) >> 1;
            if (measure(tok.text.slice(0, mid), tok.font) <= avail) low = mid;
            else high = mid - 1;
          }
          ctx.font = tok.font;
          ctx.fillStyle = toneColor(theme, tok.tone);
          ctx.fillText(`${tok.text.slice(0, low)}${ellipsis}`, x, middleY);
        } else if (tok.kind === 'missing' && avail >= MISSING_BOX_SIZE + 4) {
          drawHatchRect(ctx, x, Math.round(middleY - MISSING_BOX_SIZE / 2), MISSING_BOX_SIZE, MISSING_BOX_SIZE, 2, theme);
          x += MISSING_BOX_SIZE + 4;
          const wordAvail = limitX - x;
          let low = 0;
          let high = NOT_RECORDED_LABEL.length;
          while (low < high) {
            const mid = (low + high + 1) >> 1;
            if (measure(NOT_RECORDED_LABEL.slice(0, mid), tok.font) <= wordAvail) low = mid;
            else high = mid - 1;
          }
          ctx.font = tok.font;
          ctx.fillStyle = theme.muted;
          ctx.fillText(`${NOT_RECORDED_LABEL.slice(0, low)}${ellipsis}`, x, middleY);
        } else {
          ctx.font = baseFont;
          ctx.fillStyle = theme.muted;
          ctx.fillText(ellipsis, x, middleY);
        }
        return;
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

function bandPaintSig(band: LogBand | undefined, icons: ReadonlyMap<string, ImageBitmap | null>): string {
  if (!band) return '';
  switch (band.kind) {
    case 'text': {
      const v = band.text;
      const body = v.kind === 'known' || v.kind === 'zero' || v.kind === 'partial' ? v.value : v.kind;
      return `t:${v.kind}:${body}:${band.tone ?? ''}:${band.weight ?? ''}`;
    }
    case 'value': {
      const v = band.value;
      const body = v.kind === 'known' || v.kind === 'zero' || v.kind === 'partial' ? String(v.value) : v.kind;
      const fmt = typeof band.format === 'string'
        ? band.format
        : band.format
          ? `${band.format.decimals}:${band.format.percent ? 1 : 0}:${band.format.unit ?? ''}:${band.format.signed ? 1 : 0}:${band.format.maxDecimals ?? ''}`
          : '';
      return `v:${v.kind}:${body}:${fmt}:${band.tone ?? ''}:${band.align ?? ''}`;
    }
    case 'pill': {
      const v = band.label;
      const body = v.kind === 'known' || v.kind === 'zero' || v.kind === 'partial' ? v.value : v.kind;
      return `p:${v.kind}:${body}:${band.tone ?? ''}:${band.liveDot ? 1 : 0}`;
    }
    case 'flags': {
      let s = 'f:';
      for (let i = 0; i < band.looks.length; i++) {
        const l = band.looks[i];
        s += `${l?.name ?? ''}:${l?.flag ? (icons.get(l.flag) ? '1' : '0') : ''},`;
      }
      return s;
    }
    case 'rich': {
      let s = 'r:';
      for (let i = 0; i < band.parts.length; i++) {
        const p = band.parts[i]!;
        const v = p.text;
        const body = v.kind === 'known' || v.kind === 'zero' || v.kind === 'partial' ? v.value : v.kind;
        const fl = p.flag ? (p.flag.flag ? (icons.get(p.flag.flag) ? '1' : '0') : 'n') : '';
        s += `${fl}|${v.kind}:${body}:${p.tone ?? ''}:${p.weight ?? ''};`;
      }
      return s;
    }
    case 'meter':
    case 'bar-pair':
    case 'chips':
      return band.kind;
  }
}

function rowPaintSig(row: LogRowSpec, icons: ReadonlyMap<string, ImageBitmap | null>): string {
  const art = row.art
    ? `${row.art.icon ? (icons.get(row.art.icon) ? '1' : '0') : ''}:${row.art.glyph ?? ''}:${row.art.color ?? ''}`
    : '';
  let sig = `${row.key}@${row.hour}:${row.by ? 1 : 0}:${art}`;
  for (let i = 0; i < row.slots.length; i++) {
    sig += `|${bandPaintSig(row.slots[i], icons)}`;
  }
  if (row.subline) {
    sig += `^${bandPaintSig(row.subline, icons)}`;
  }
  return sig;
}

function paintItem(
  ctx: ValueCanvasContext,
  picture: LogPicture,
  item: FlatLogItem,
  y: number,
  width: number,
  density: LogDensity,
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

  const middleY = y + h / 2;
  if (dateCol) {
    const parts = shortDateParts(row.hour);
    const dayText = row.by ? `by ${parts.day}` : parts.day;
    const dayFont = fontOf(theme, theme.fontSize.small, LABEL_WEIGHT);
    const yearFont = fontOf(theme, Math.max(10, theme.fontSize.small - 1), LABEL_WEIGHT);
    ctx.textBaseline = 'middle';
    ctx.textAlign = 'left';
    if (density === 'comfortable') {
      ctx.font = dayFont;
      ctx.fillStyle = theme.muted;
      ctx.fillText(fitText(ctx, picture, dayText, dayFont, dateCol.width), dateCol.left, y + h * 0.34);
      ctx.font = yearFont;
      ctx.fillStyle = theme.muted;
      const curAlpha = ctx.globalAlpha;
      ctx.globalAlpha = curAlpha * 0.8;
      ctx.fillText(parts.year, dateCol.left, y + h * 0.7);
      ctx.globalAlpha = curAlpha;
    } else {
      const line = `${dayText} ${parts.year}`;
      ctx.font = dayFont;
      ctx.fillStyle = theme.muted;
      ctx.fillText(fitText(ctx, picture, line, dayFont, dateCol.width), dateCol.left, middleY);
    }
  }

  if (artCol) {
    drawArt(ctx, row, artCol, middleY, theme, icons);
  }

  const compactSideBySide = density === 'compact' && twoLine;
  for (let s = 0; s < contentCols.length && s < row.slots.length; s++) {
    const col = contentCols[s]!;
    const band = row.slots[s]!;
    if (s === 0 && twoLine && row.subline) {
      if (compactSideBySide) {
        if (band.kind === 'text') {
          const formatted = formatValueState(band.text);
          if (formatted.kind === 'known' || formatted.kind === 'zero') {
            const titleFont = fontOf(theme, theme.fontSize.small, band.weight === 'bold' ? VALUE_WEIGHT : LABEL_WEIGHT);
            const fullTitleW = measureText(ctx, picture, formatted.text, titleFont);
            const gapPx = 8;
            const minSublineW = 48;
            const maxTitleW = fullTitleW + gapPx + minSublineW <= col.width
              ? fullTitleW
              : Math.min(fullTitleW, Math.max(0, col.width - gapPx - minSublineW));
            const titleW = maxTitleW >= 24 ? maxTitleW : col.width;
            const fittedTitle = fitText(ctx, picture, formatted.text, titleFont, titleW);
            if (fittedTitle) {
              ctx.font = titleFont;
              ctx.fillStyle = toneColor(theme, band.tone);
              ctx.textBaseline = 'middle';
              ctx.textAlign = 'left';
              ctx.fillText(fittedTitle, col.left, middleY);
            }
            const usedW = fittedTitle ? measureText(ctx, picture, fittedTitle, titleFont) : 0;
            const subLeft = col.left + usedW + gapPx;
            const subWidth = col.left + col.width - subLeft;
            if (subWidth > 8) {
              drawBand(ctx, picture, row.subline, subLeft, subWidth, middleY, 'start', true, theme, icons);
            }
          } else {
            drawBand(ctx, picture, band, col.left, col.width, middleY, col.align, true, theme, icons);
          }
        } else {
          drawBand(ctx, picture, band, col.left, col.width, middleY, col.align, true, theme, icons);
        }
      } else {
        drawBand(ctx, picture, band, col.left, col.width, y + h * 0.34, col.align, false, theme, icons);
        drawBand(ctx, picture, row.subline, col.left, col.width, y + h * 0.72, col.align, true, theme, icons);
      }
    } else {
      drawBand(ctx, picture, band, col.left, col.width, middleY, col.align, density === 'compact', theme, icons);
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
    preparePrevCanvas,
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
      ? `${rowPaintSig(item.row, icons)}#${hl}${alphaPart}`
      : item.kind === 'month'
        ? `m:${item.label}`
        : `g:${item.label}#${hl}`;
    visSigs.push(base);
  }
  const scannedItems = visKeys.length;

  const themeKey = [
    theme.surface,
    theme.surfaceAlt,
    theme.hover,
    theme.selected,
    theme.text,
    theme.muted,
    theme.line,
    theme.lineSoft,
    theme.accent,
    theme.positive,
    theme.negative,
    theme.warning,
    theme.hatch,
    theme.hatchEdge ?? '',
    String(theme.hatchSpacing),
    theme.font,
    String(theme.fontSize.small),
    String(theme.fontSize.body),
    theme.categories.join(','),
  ].join('|');
  const geomKey = `${width}x${height}@${dpr}|${density}|${showMonths ? 1 : 0}|${model.twoLine ? 1 : 0}|${themeKey}`;
  const sig = `${geomKey}|${sticky ?? ''}|${visKeys.length}|${visTops.join(',')}|${visKeys.join(',')}|${visSigs.join(';')}`;

  if (picture.signature === sig) {
    return { drew: false, incremental: false, auditRows, paintedRows: 0, paintedItems: 0, scannedItems, visitedItems: 0, visibleStart: start, visibleEnd: end };
  }

  const prev = picture.lastSlice;
  const hasPrev = prevCanvas !== undefined ? prevCanvas !== null : preparePrevCanvas !== undefined;
  let shiftPrefix = -1;
  let shiftPrevAnchor = 0;
  if (prev && hasPrev && prev.geomKey === geomKey && prev.stickyH === stickyH && prev.keys.length > 0 && visKeys.length > 1) {
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

  const columns = resolveLogColumns(model.columns, width, 12, 10);
  const colById = new Map<string, ResolvedLogColumn>();
  for (const col of columns) colById.set(col.id, col);
  const dateCol = colById.get('date');
  const artCol = colById.get('art');
  const contentCols = columns.filter(col => col.id !== 'date' && col.id !== 'art');
  const twoLine = model.twoLine === true;

  let paintedRows = 0;
  let paintedItems = 0;
  let incremental = false;

  ctx.save();
  if (shiftPrefix > 0 && prev) {
    const copyStartLogical = prev.tops[shiftPrevAnchor]!;
    const dstStartLogical = visTops[shiftPrefix]!;
    const srcY = Math.round(copyStartLogical * dpr);
    const dstY = Math.round(dstStartLogical * dpr);
    const physW = Math.round(width * dpr);
    const physH = Math.round(height * dpr);
    const copyH = physH - dstY;
    const blitSource = srcY >= 0 && dstY > srcY && copyH > 0 && srcY + copyH <= physH
      ? (prevCanvas ?? preparePrevCanvas?.() ?? null)
      : null;
    if (blitSource) {
      incremental = true;
      ctx.setTransform(1, 0, 0, 1, 0, 0);
      ctx.fillStyle = theme.surface;
      ctx.fillRect(0, 0, physW, dstY);
      ctx.drawImage(blitSource, 0, srcY, physW, copyH, 0, dstY, physW, copyH);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

      let visIdx = 0;
      for (let i = start; i < end && visIdx < shiftPrefix; i++) {
        const item = layout.items[i]!;
        const y = Math.round(item.top - scrollTop);
        if (y + item.height <= stickyH || y >= height) continue;
        const alpha = item.kind === 'row' && rowAlphas ? (rowAlphas.get(item.row.key) ?? 1) : 1;
        paintItem(ctx, picture, item, y, width, density, twoLine, dateCol, artCol, contentCols, theme, inspectedKey, focusedKey, selectedKey, icons, alpha);
        paintedItems++;
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
      paintItem(ctx, picture, item, y, width, density, twoLine, dateCol, artCol, contentCols, theme, inspectedKey, focusedKey, selectedKey, icons, alpha);
      paintedItems++;
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
  return { drew: true, incremental, auditRows, paintedRows, paintedItems, scannedItems, visitedItems: paintedItems, visibleStart: start, visibleEnd: end };
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
