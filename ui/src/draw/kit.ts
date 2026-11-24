import { LABEL_WEIGHT, VALUE_WEIGHT } from '../charts/draw.ts';
import {
  drawFittedLabel,
  ellipsise,
  fitLine,
  graphemes,
  labelFitsTwoLines,
  labelLineHeight,
  type FittedLabel,
  type FittedLine,
} from '../charts/labels.ts';
import { textWidth, type Canvas2D } from '../charts/textWidth.ts';
import type { FamilyTheme } from '../families/contract';
import {
  drawHatchRect,
  MISSING_BOX_GAP,
  MISSING_BOX_SIZE,
  PARTIAL_TAB_HEIGHT,
  PARTIAL_TAB_WIDTH,
} from '../families/valueCanvas.ts';
import {
  formatDateOnly,
  formatDateParts,
  GAME_WINDOW_MISSING_LABEL,
  NOT_APPLICABLE_LABEL,
  notRecordedLabel,
} from '../families/valueFormat.ts';
import { CHART_MEASURE_COLOURS } from '../charts/palette.ts';
import { luminance, mix, parseColor, rgbaColor } from '../shell/appearance/color.ts';
import { engineHour, gameDate, HOUR_EPOCH, monthNames } from '../dashboard/calendar.ts';
import type { KitRaster, PathBuilder, PathPaint } from './path';

export type Align = 'left' | 'right' | 'center';
type Baseline = 'top' | 'middle' | 'bottom' | 'alphabetic';
export interface LabelStyle { font: string; fill: string; align: Align; baseline: Baseline; squeeze?: boolean }

export interface Kit {
  begin(width: number, height: number, background?: string): void;
  rect(x: number, y: number, width: number, height: number, fill: string, radius?: number): void;
  rule(x0: number, y0: number, x1: number, y1: number, stroke: string, width?: number): void;
  area(xs: ArrayLike<number>, highs: ArrayLike<number>, lows: ArrayLike<number>, count: number, fill: string, outline?: string): void;
  line(xs: ArrayLike<number>, ys: ArrayLike<number>, count: number, stroke: string, width: number): void;
  label(text: string, x: number, y: number, style: LabelStyle, maxWidth?: number): number;
  measure(text: string, font: string): number;
  icon(key: string, x: number, y: number, width: number, height: number): boolean;
  fade(alpha: number): void;
  dashed(pattern: readonly number[]): void;
  desaturate(): void;
  clip(x: number, y: number, width: number, height: number): void;
  newPath(): PathBuilder;
  clipPath(path: PathBuilder): void;
  path(path: PathBuilder, paint: PathPaint): void;
  erase(x: number, y: number, width: number, height: number): void;
  raster(raster: KitRaster, sx: number, sy: number, sw: number, sh: number, dx: number, dy: number, dw: number, dh: number): void;
  readonly pixelRatio: number;
  save(): void;
  restore(): void;
  place?(axes: Placement): void;
  when?(span: readonly [number, number] | null): void;
  pulse?(x: number, y: number, size: number, fill: string): void;
}

export type Placement = 'pixel' | 'x' | 'y' | 'xy';

export interface DrawRegion {
  x: number; y: number; width: number; height: number;
  clip: [left: number, top: number, right: number, bottom: number];
}

export type RowDensity = 'compact' | 'normal' | 'comfortable';

export const ROW_DENSITY_HEIGHT: Readonly<Record<RowDensity, number>> = {
  compact: 28,
  normal: 36,
  comfortable: 44,
};

export const PORTRAIT_DENSITY_HEIGHT: Readonly<Record<RowDensity, number>> = {
  compact: 40,
  normal: 48,
  comfortable: 58,
};

export const BEHIND_BAR_HEIGHT: Readonly<Record<RowDensity, number>> = {
  compact: 18,
  normal: 22,
  comfortable: 26,
};

export const BEHIND_BAR_MAX_WIDTH = 64;
export const BEHIND_BAR_MIN_WIDTH = 6;
export const BEHIND_BAR_OFFSET = 7;
export const ROW_CORNER_RADIUS = 7;
export const FLAG_SLOT_WIDTH = 20;
export const FLAG_SLOT_HEIGHT = 13;
export const FLAG_LOSER_ALPHA = 0.35;
export const DATE_COLUMN_WIDTH = 50;
export const SERIES_MISSING_STRIP_HEIGHT = 5;
export const TRAIT_ICON_SIZE = 23;
export const MEDAL_ICON_SIZE = 28;
export const ICON_STRIP_GAP = 5;

const INITIALS_CACHE_MAX = 1024;
const initialsCache = new Map<string, string>();

function isDarkTheme(theme: Pick<FamilyTheme, 'surface' | 'dark'>): boolean {
  if (theme.dark !== undefined) return theme.dark;
  const rgb = parseColor(theme.surface);
  if (!rgb) return true;
  return 0.2126 * rgb.r + 0.7152 * rgb.g + 0.0722 * rgb.b < 128;
}

export const DEFAULT_FAMILY_THEME: FamilyTheme = {
  surface: '#111111',
  raised: '#262626',
  dark: true,
  text: '#f5f5f5',
  muted: '#929292',
  line: 'rgba(245, 245, 245, 0.18)',
  lineSoft: 'rgba(245, 245, 245, 0.055)',
  accent: '#4f86f7',
  blue: '#4f86f7',
  red: CHART_MEASURE_COLOURS.short,
  shortage: CHART_MEASURE_COLOURS.shortInk,
  amber: CHART_MEASURE_COLOURS.politicalPower,
  green: CHART_MEASURE_COLOURS.supplyOk,
  grey: '#929292',
  hatch: 'rgba(245, 245, 245, 0.17)',
  hatchEdge: 'rgba(245, 245, 245, 0.12)',
  font: "'Segoe UI', system-ui, sans-serif",
  fontSize: { small: 11, body: 12 },
  lineWidth: 1,
  hatchSpacing: 5,
};

export function resolveKitTokens(
  theme: Pick<FamilyTheme, 'surface' | 'text' | 'muted' | 'accent'> & Partial<FamilyTheme>,
) {
  const dark = isDarkTheme(theme);
  const ink = dark ? '255, 255, 255' : '15, 23, 42';
  return {
    dark,
    raised: theme.raised ?? theme.surface,
    hairline: theme.hairline ?? `rgba(${ink}, ${dark ? 0.05 : 0.07})`,
    hairlineStrong: theme.hairlineStrong ?? `rgba(${ink}, ${dark ? 0.085 : 0.11})`,
    bandDivider: theme.bandDivider ?? `rgba(${ink}, ${dark ? 0.12 : 0.15})`,
    headerBand: theme.headerBand ?? `rgba(${ink}, ${dark ? 0.025 : 0.03})`,
    rowHover: theme.rowHover ?? `rgba(${ink}, ${dark ? 0.045 : 0.05})`,
    rowSelected: theme.rowSelected ?? rgbaColor(theme.accent, 0.14, dark ? 'rgba(96, 165, 250, 0.14)' : 'rgba(37, 99, 235, 0.14)'),
    childTint: theme.childTint ?? `rgba(${ink}, ${dark ? 0.018 : 0.025})`,
    shortage: theme.shortage ?? (dark ? '#ff6b6f' : '#e5484d'),
    barMuted: theme.barMuted ?? `rgba(${ink}, ${dark ? 0.085 : 0.095})`,
    barTrack: theme.barTrack ?? `rgba(${ink}, ${dark ? 0.08 : 0.1})`,
    gridLine: theme.gridLine ?? `rgba(${ink}, ${dark ? 0.055 : 0.07})`,
    zeroLine: theme.zeroLine ?? `rgba(${ink}, ${dark ? 0.18 : 0.22})`,
    tailLine: theme.tailLine ?? rgbaColor(theme.muted, 0.22, `rgba(${ink}, 0.22)`),
    playhead: theme.playhead ?? `rgba(${ink}, ${dark ? 0.28 : 0.34})`,
    crosshair: theme.crosshair ?? `rgba(${ink}, ${dark ? 0.5 : 0.52})`,
    dateTabBg: theme.dateTabBg ?? (theme.raised ?? theme.surface),
    dateTabText: theme.dateTabText ?? theme.text,
    hoverTabBg: theme.hoverTabBg ?? theme.text,
    hoverTabText: theme.hoverTabText ?? theme.surface,
    textSecondary: dark ? '#d4d4d4' : theme.muted,
    autoName: mix(theme.text, theme.surface, 0.25),
    childText: dark ? '#d4d4d4' : theme.text,
    barNeutral: dark ? '#cfcfcf' : '#64748b',
    trackGrey: dark ? '#c4c4c4' : '#64748b',
    twoWayNeutral: dark ? 'rgba(245, 245, 245, 0.28)' : 'rgba(15, 23, 42, 0.25)',
    markerTick: dark ? 'rgba(245, 245, 245, 0.42)' : 'rgba(15, 23, 42, 0.42)',
    partsTrack: dark ? 'rgba(245, 245, 245, 0.045)' : 'rgba(15, 23, 42, 0.05)',
  };
}

export function drawRowHover(
  ctx: Canvas2D,
  x: number,
  y: number,
  width: number,
  height: number,
  theme: FamilyTheme,
  inset = 4,
): void {
  const w = width - inset * 2;
  const h = height - 2;
  if (w <= 0 || h <= 0) return;
  const tokens = resolveKitTokens(theme);
  ctx.save();
  ctx.fillStyle = tokens.rowHover;
  ctx.beginPath();
  ctx.roundRect(x + inset, y + 1, w, h, ROW_CORNER_RADIUS);
  ctx.fill();
  ctx.restore();
}

export function drawRowSelected(
  ctx: Canvas2D,
  x: number,
  y: number,
  width: number,
  height: number,
  theme: FamilyTheme,
  inset = 4,
): void {
  const w = width - inset * 2;
  const h = height - 2;
  if (w <= 0 || h <= 0) return;
  const tokens = resolveKitTokens(theme);
  ctx.save();
  ctx.fillStyle = tokens.rowSelected;
  ctx.beginPath();
  ctx.roundRect(x + inset, y + 1, w, h, ROW_CORNER_RADIUS);
  ctx.fill();
  ctx.restore();
}

export function drawRowFocusRing(
  ctx: Canvas2D,
  x: number,
  y: number,
  width: number,
  height: number,
  theme: FamilyTheme,
  inset = 4,
): void {
  const w = width - inset * 2 - 2;
  const h = height - 4;
  if (w <= 0 || h <= 0) return;
  ctx.save();
  ctx.strokeStyle = theme.accent;
  ctx.lineWidth = 2;
  ctx.beginPath();
  ctx.roundRect(x + inset + 1, y + 2, w, h, ROW_CORNER_RADIUS);
  ctx.stroke();
  ctx.restore();
}

export function drawRowHairline(
  ctx: Canvas2D,
  x0: number,
  x1: number,
  y: number,
  theme: FamilyTheme,
  kind: 'normal' | 'strong' | 'band' = 'normal',
): void {
  if (x1 <= x0) return;
  const tokens = resolveKitTokens(theme);
  const stroke = kind === 'band' ? tokens.bandDivider : kind === 'strong' ? tokens.hairlineStrong : tokens.hairline;
  const py = Math.floor(y) + 0.5;
  ctx.save();
  ctx.strokeStyle = stroke;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(x0, py);
  ctx.lineTo(x1, py);
  ctx.stroke();
  ctx.restore();
}

export function drawChildRowTint(
  ctx: Canvas2D,
  x: number,
  y: number,
  width: number,
  height: number,
  theme: FamilyTheme,
): void {
  if (width <= 0 || height <= 0) return;
  const tokens = resolveKitTokens(theme);
  ctx.save();
  ctx.fillStyle = tokens.childTint;
  ctx.fillRect(x, y, width, height);
  ctx.restore();
}

export function drawScrollFade(
  ctx: Canvas2D,
  x: number,
  y: number,
  width: number,
  height: number,
  theme: FamilyTheme,
  edge: 'top' | 'bottom' | 'left' | 'right',
): void {
  if (width <= 0 || height <= 0) return;
  const x0 = edge === 'right' ? x + width : x;
  const y0 = edge === 'bottom' ? y + height : y;
  const x1 = edge === 'left' ? x + width : edge === 'right' ? x : x;
  const y1 = edge === 'top' ? y + height : edge === 'bottom' ? y : y;
  ctx.save();
  const gradient = ctx.createLinearGradient(x0, y0, x1, y1);
  gradient.addColorStop(0, theme.surface);
  gradient.addColorStop(1, rgbaColor(theme.surface, 0, 'rgba(0, 0, 0, 0)'));
  ctx.fillStyle = gradient;
  ctx.fillRect(x, y, width, height);
  ctx.restore();
}

export function artInitials(resolvedName: string | undefined): string {
  if (!resolvedName) return '';
  const trimmed = resolvedName.trim();
  if (!trimmed) return '';
  const cached = initialsCache.get(trimmed);
  if (cached !== undefined) return cached;
  const words = trimmed.split(/[\s\-_/·]+/).filter(Boolean);
  const letters: string[] = [];
  for (const word of words) {
    const first = graphemes(word).find(g => /[\p{L}\p{N}]/u.test(g));
    if (first) letters.push(first.toLocaleUpperCase());
    if (letters.length === 3) break;
  }
  if (!letters.length) {
    for (const g of graphemes(trimmed)) {
      if (/[\p{L}\p{N}]/u.test(g)) {
        letters.push(g.toLocaleUpperCase());
        if (letters.length === 2) break;
      }
    }
  }
  const result = letters.join('');
  if (initialsCache.size >= INITIALS_CACHE_MAX) initialsCache.clear();
  initialsCache.set(trimmed, result);
  return result;
}

export type ArtSlotState =
  | { readonly kind: 'none' }
  | { readonly kind: 'pending' }
  | { readonly kind: 'ready'; readonly bitmap: CanvasImageSource; readonly width: number; readonly height: number }
  | { readonly kind: 'missing'; readonly resolvedName?: string }
  | { readonly kind: 'refused'; readonly reason?: string }
  | { readonly kind: 'failed'; readonly reason?: string };

export interface ArtSlotResult {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
  readonly drawn: ArtSlotState['kind'];
  readonly initials?: string;
}

export function drawArtSlot(
  ctx: Canvas2D,
  x: number,
  y: number,
  width: number,
  height: number,
  state: ArtSlotState,
  theme: FamilyTheme,
  options?: { readonly radius?: number; readonly alpha?: number },
): ArtSlotResult {
  const radius = options?.radius ?? 3;
  if (width <= 0 || height <= 0 || state.kind === 'none' || state.kind === 'pending') {
    return { x, y, width, height, drawn: state.kind };
  }
  if (state.kind === 'ready') {
    const sw = Math.max(1, state.width);
    const sh = Math.max(1, state.height);
    const scale = Math.min(width / sw, height / sh);
    const dw = sw * scale;
    const dh = sh * scale;
    const dx = x + (width - dw) / 2;
    const dy = y + (height - dh) / 2;
    ctx.save();
    if (options?.alpha !== undefined) ctx.globalAlpha = options.alpha;
    if (radius > 0) {
      ctx.beginPath();
      ctx.roundRect(x, y, width, height, radius);
      ctx.clip();
    }
    ctx.drawImage(state.bitmap, dx, dy, dw, dh);
    ctx.restore();
    return { x, y, width, height, drawn: 'ready' };
  }
  if (state.kind === 'missing') {
    ctx.save();
    if (options?.alpha !== undefined) ctx.globalAlpha = options.alpha;
    drawHatchRect(ctx, x, y, width, height, radius, theme);
    const initials = Math.min(width, height) > 24 ? artInitials(state.resolvedName) : '';
    if (initials) {
      const size = Math.max(9, Math.min(13, Math.round(Math.min(width, height) * 0.34)));
      ctx.font = `${VALUE_WEIGHT} ${size}px ${theme.font}`;
      ctx.fillStyle = theme.muted;
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      const shown = ellipsise(initials, Math.max(1, width - 4), s => textWidth(ctx, s));
      ctx.fillText(shown, x + width / 2, y + height / 2);
    }
    ctx.restore();
    return { x, y, width, height, drawn: 'missing', ...(initials ? { initials } : {}) };
  }
  const tokens = resolveKitTokens(theme);
  ctx.save();
  if (options?.alpha !== undefined) ctx.globalAlpha = options.alpha;
  ctx.fillStyle = tokens.headerBand;
  ctx.beginPath();
  ctx.roundRect(x, y, width, height, radius);
  ctx.fill();
  ctx.strokeStyle = theme.line;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.roundRect(x + 0.5, y + 0.5, Math.max(0, width - 1), Math.max(0, height - 1), radius);
  ctx.stroke();
  ctx.fillStyle = theme.muted;
  ctx.beginPath();
  ctx.arc(x + width / 2, y + height / 2, Math.max(1.5, Math.min(2.5, Math.min(width, height) * 0.12)), 0, Math.PI * 2);
  ctx.fill();
  ctx.restore();
  return { x, y, width, height, drawn: state.kind };
}

export type FlagSlotState =
  | { readonly kind: 'none' }
  | { readonly kind: 'pending' }
  | { readonly kind: 'ready'; readonly bitmap: CanvasImageSource; readonly width: number; readonly height: number }
  | { readonly kind: 'missing'; readonly tag?: string }
  | { readonly kind: 'refused'; readonly reason?: string }
  | { readonly kind: 'failed' };

export interface FlagSlotResult {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
  readonly drawn: FlagSlotState['kind'];
  readonly tag?: string;
}

export function drawFlagSlot(
  ctx: Canvas2D,
  x: number,
  y: number,
  state: FlagSlotState,
  theme: FamilyTheme,
  options?: { readonly width?: number; readonly height?: number; readonly dimmed?: boolean; readonly radius?: number },
): FlagSlotResult {
  const width = options?.width ?? FLAG_SLOT_WIDTH;
  const height = options?.height ?? FLAG_SLOT_HEIGHT;
  const radius = options?.radius ?? 2;
  const alpha = options?.dimmed ? FLAG_LOSER_ALPHA : 1;
  if (width <= 0 || height <= 0 || state.kind === 'pending' || state.kind === 'none') {
    return { x, y, width, height, drawn: state.kind };
  }
  if (state.kind === 'ready') {
    const sw = Math.max(1, state.width);
    const sh = Math.max(1, state.height);
    const scale = Math.min(width / sw, height / sh);
    const dw = sw * scale;
    const dh = sh * scale;
    const dx = x + (width - dw) / 2;
    const dy = y + (height - dh) / 2;
    ctx.save();
    ctx.globalAlpha = alpha;
    if (radius > 0) {
      ctx.beginPath();
      ctx.roundRect(x, y, width, height, radius);
      ctx.clip();
    }
    ctx.drawImage(state.bitmap, dx, dy, dw, dh);
    ctx.restore();
    ctx.save();
    ctx.globalAlpha = alpha * 0.4;
    ctx.strokeStyle = theme.line;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.roundRect(x + 0.5, y + 0.5, Math.max(0, width - 1), Math.max(0, height - 1), radius);
    ctx.stroke();
    ctx.restore();
    return { x, y, width, height, drawn: 'ready' };
  }
  if (state.kind === 'missing') {
    const tag = state.tag?.trim().toUpperCase().slice(0, 3) || '?';
    ctx.save();
    ctx.globalAlpha = alpha;
    drawHatchRect(ctx, x, y, width, height, radius, theme);
    if (width >= 16 && height >= 11) {
      ctx.font = `${VALUE_WEIGHT} ${Math.max(7, Math.min(9, height - 4))}px ${theme.font}`;
      ctx.fillStyle = theme.muted;
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillText(ellipsise(tag, Math.max(1, width - 2), s => textWidth(ctx, s)), x + width / 2, y + height / 2 + 0.5);
    }
    ctx.restore();
    return { x, y, width, height, drawn: 'missing', tag };
  }
  ctx.save();
  ctx.globalAlpha = alpha;
  ctx.strokeStyle = theme.line;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.roundRect(x + 0.5, y + 0.5, Math.max(0, width - 1), Math.max(0, height - 1), radius);
  ctx.stroke();
  ctx.restore();
  return { x, y, width, height, drawn: state.kind };
}

export function drawNameSlot(
  ctx: Canvas2D,
  text: string,
  x: number,
  y: number,
  width: number,
  rowHeight: number,
  theme: FamilyTheme,
  options?: {
    readonly fontSize?: number;
    readonly weight?: number;
    readonly color?: string;
    readonly align?: CanvasTextAlign;
    readonly maxLines?: 1 | 2;
    readonly lineHeight?: number;
  },
): FittedLabel {
  const fontSize = options?.fontSize ?? theme.fontSize.body;
  const weight = options?.weight ?? LABEL_WEIGHT;
  const lineHeight = options?.lineHeight ?? labelLineHeight(fontSize);
  const maxLines: 1 | 2 = options?.maxLines ?? (labelFitsTwoLines(rowHeight, lineHeight) ? 2 : 1);
  ctx.save();
  ctx.font = `${weight} ${fontSize}px ${theme.font}`;
  ctx.fillStyle = options?.color ?? theme.text;
  ctx.textAlign = options?.align ?? 'left';
  ctx.textBaseline = 'middle';
  const fitted = drawFittedLabel(ctx, text, x, y + rowHeight / 2, width, {
    fontSize,
    lineHeight,
    maxLines,
  });
  ctx.restore();
  return fitted;
}

export function barCornerRadius(height: number): number {
  return Math.max(0, Math.min(4, height / 3));
}

export function barEndPath(
  ctx: Canvas2D,
  x: number,
  y: number,
  width: number,
  height: number,
  direction: 'right' | 'left' = 'right',
  radius = barCornerRadius(height),
): void {
  const r = Math.min(radius, Math.max(0, width), Math.max(0, height / 2));
  const radii: [number, number, number, number] = direction === 'right' ? [0, r, r, 0] : [r, 0, 0, r];
  ctx.beginPath();
  ctx.roundRect(x, y, width, height, radii);
}

export function drawBar(
  ctx: Canvas2D,
  x: number,
  y: number,
  width: number,
  height: number,
  fill: string,
  direction: 'right' | 'left' = 'right',
  alpha = 1,
): void {
  if (width <= 0 || height <= 0) return;
  ctx.save();
  ctx.fillStyle = fill;
  ctx.globalAlpha = alpha;
  barEndPath(ctx, x, y, width, height, direction);
  ctx.fill();
  ctx.restore();
}

export function drawBarBehindFigure(
  ctx: Canvas2D,
  figureRight: number,
  middleY: number,
  fraction: number,
  fill: string,
  options?: {
    readonly density?: RowDensity;
    readonly sorted?: boolean;
    readonly maxWidth?: number;
    readonly theme?: FamilyTheme;
  },
): { readonly x: number; readonly y: number; readonly width: number; readonly height: number } | null {
  const clamped = Math.max(0, Math.min(1, fraction));
  if (clamped <= 0) return null;
  const density = options?.density ?? 'normal';
  const height = BEHIND_BAR_HEIGHT[density];
  const maxWidth = options?.maxWidth ?? BEHIND_BAR_MAX_WIDTH;
  const width = Math.min(maxWidth, Math.max(BEHIND_BAR_MIN_WIDTH, Math.round(maxWidth * clamped)));
  const startRight = figureRight + BEHIND_BAR_OFFSET;
  const x = startRight - width;
  const y = Math.round(middleY - height / 2);
  const sorted = options?.sorted ?? true;
  if (sorted) {
    drawBar(ctx, x, y, width, height, fill, 'left', 0.3);
  } else {
    const mutedFill = options?.theme ? resolveKitTokens(options.theme).barMuted : 'rgba(255, 255, 255, 0.085)';
    drawBar(ctx, x, y, width, height, mutedFill, 'left', 1);
  }
  return { x, y, width, height };
}

export function drawTwoWayBar(
  ctx: Canvas2D,
  centerX: number,
  middleY: number,
  halfWidth: number,
  height: number,
  signedFraction: number,
  positiveFill: string,
  negativeFill: string,
  theme: FamilyTheme,
): void {
  if (halfWidth <= 0 || height <= 0) return;
  const tokens = resolveKitTokens(theme);
  const y = Math.round(middleY - height / 2);
  const clamped = Math.max(-1, Math.min(1, signedFraction));
  const px = Math.floor(centerX) + 0.5;
  ctx.save();
  ctx.strokeStyle = tokens.zeroLine;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(px, y - 2);
  ctx.lineTo(px, y + height + 2);
  ctx.stroke();
  ctx.restore();
  if (clamped > 0) {
    drawBar(ctx, centerX, y, halfWidth * clamped, height, positiveFill, 'right');
  } else if (clamped < 0) {
    const w = halfWidth * Math.abs(clamped);
    drawBar(ctx, centerX - w, y, w, height, negativeFill, 'left');
  }
}

export interface SplitBarSegment {
  readonly fraction: number;
  readonly fill: string;
  readonly hatched?: boolean;
}

export function drawSplitBar(
  ctx: Canvas2D,
  x: number,
  y: number,
  width: number,
  height: number,
  segments: readonly SplitBarSegment[],
  theme: FamilyTheme,
): void {
  if (width <= 0 || height <= 0) return;
  const active = segments.filter(s => s.fraction > 0);
  if (!active.length) return;
  const radius = barCornerRadius(height);
  let cursor = x;
  for (let i = 0; i < active.length; i++) {
    const segment = active[i]!;
    const segW = Math.min(x + width - cursor, width * segment.fraction);
    if (segW <= 0) continue;
    const isLast = i === active.length - 1;
    if (segment.hatched) {
      drawHatchRect(ctx, cursor, y, segW, height, isLast ? radius : 0, theme);
    } else if (isLast) {
      drawBar(ctx, cursor, y, segW, height, segment.fill, 'right');
    } else {
      ctx.save();
      ctx.fillStyle = segment.fill;
      ctx.fillRect(cursor, y, segW, height);
      ctx.restore();
    }
    cursor += segW;
  }
}

export function drawTrackBar(
  ctx: Canvas2D,
  x: number,
  middleY: number,
  width: number,
  height: number,
  fraction: number,
  fill: string,
  theme: FamilyTheme,
): void {
  if (width <= 0 || height <= 0) return;
  const tokens = resolveKitTokens(theme);
  const y = Math.round(middleY - height / 2);
  const clamped = Math.max(0, Math.min(1, fraction));
  drawBar(ctx, x, y, width, height, tokens.barTrack, 'right');
  if (clamped > 0) {
    drawBar(ctx, x, y, width * clamped, height, fill, 'right');
  }
}

export interface ThresholdTick {
  readonly at: number;
  readonly reached: boolean;
}

export function drawThresholdMeter(
  ctx: Canvas2D,
  x: number,
  middleY: number,
  width: number,
  height: number,
  fraction: number | null,
  ticks: readonly ThresholdTick[],
  fill: string,
  theme: FamilyTheme,
): void {
  if (width <= 0 || height <= 0) return;
  const y = Math.round(middleY - height / 2);
  if (fraction === null) {
    drawHatchRect(ctx, x, y, width, height, barCornerRadius(height), theme);
    return;
  }
  drawTrackBar(ctx, x, middleY, width, height, fraction, fill, theme);
  if (!ticks.length) return;
  ctx.save();
  ctx.lineWidth = 1.2;
  for (let i = 0; i < ticks.length; i++) {
    const tick = ticks[i]!;
    if (tick.at <= 0 || tick.at >= 1) continue;
    const tx = Math.round(x + width * tick.at) + 0.5;
    ctx.strokeStyle = theme.muted;
    ctx.globalAlpha = tick.reached ? 0.9 : 0.55;
    ctx.beginPath();
    ctx.moveTo(tx, y - 3);
    ctx.lineTo(tx, y + height + 3);
    ctx.stroke();
  }
  ctx.restore();
}

export function drawProgressRing(
  ctx: Canvas2D,
  cx: number,
  cy: number,
  radius: number,
  fraction: number,
  stroke: string,
  theme: FamilyTheme,
  lineWidth = 2.4,
): void {
  if (radius <= 0) return;
  const tokens = resolveKitTokens(theme);
  const clamped = Math.max(0, Math.min(1, fraction));
  ctx.save();
  ctx.lineWidth = lineWidth;
  ctx.strokeStyle = tokens.barTrack;
  ctx.beginPath();
  ctx.arc(cx, cy, radius, 0, Math.PI * 2);
  ctx.stroke();
  if (clamped > 0) {
    const start = -Math.PI / 2;
    const end = start + clamped * Math.PI * 2;
    ctx.strokeStyle = stroke;
    ctx.lineCap = 'round';
    ctx.beginPath();
    ctx.arc(cx, cy, radius, start, end);
    ctx.stroke();
  }
  ctx.restore();
}

export type StatusMarkKind = 'running' | 'waiting' | 'succeeded' | 'finished' | 'ended';

export const STATUS_MARK_LABELS: Readonly<Record<StatusMarkKind, string>> = {
  running: 'Running',
  waiting: 'Waiting',
  succeeded: 'Succeeded',
  finished: 'Finished',
  ended: 'Ended',
};

export function drawStatusMark(
  ctx: Canvas2D,
  x: number,
  middleY: number,
  kind: StatusMarkKind,
  theme: FamilyTheme,
  options?: { readonly label?: string; readonly maxWidth?: number },
): { readonly width: number; readonly label: FittedLabel } {
  const dotRadius = 3;
  const dotDiameter = dotRadius * 2;
  const gap = 6;
  const color = kind === 'running' || kind === 'waiting'
    ? theme.accent
    : kind === 'succeeded'
      ? theme.green
      : theme.muted;
  const ring = kind === 'waiting' || kind === 'ended';
  ctx.save();
  ctx.beginPath();
  ctx.arc(x + dotRadius, middleY, ring ? dotRadius - 0.6 : dotRadius, 0, Math.PI * 2);
  if (ring) {
    ctx.strokeStyle = color;
    ctx.lineWidth = 1.4;
    ctx.stroke();
  } else {
    ctx.fillStyle = color;
    ctx.fill();
  }
  const word = options?.label ?? STATUS_MARK_LABELS[kind];
  const fontSize = theme.fontSize.small;
  ctx.font = `${LABEL_WEIGHT} ${fontSize}px ${theme.font}`;
  ctx.fillStyle = kind === 'ended' ? theme.muted : theme.text;
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  const textMax = options?.maxWidth !== undefined ? Math.max(0, options.maxWidth - dotDiameter - gap) : 10_000;
  const fitted = drawFittedLabel(ctx, word, x + dotDiameter + gap, middleY, textMax, {
    fontSize,
    lineHeight: labelLineHeight(fontSize),
    maxLines: 1,
  });
  const drawnW = fitted.lines[0] ? textWidth(ctx, fitted.lines[0]) : 0;
  ctx.restore();
  return { width: dotDiameter + gap + drawnW, label: fitted };
}

export function drawLegendEntry(
  ctx: Canvas2D,
  x: number,
  middleY: number,
  label: string,
  color: string,
  theme: FamilyTheme,
  options?: {
    readonly dash?: readonly number[];
    readonly hatch?: boolean;
    readonly outlined?: boolean;
    readonly flag?: FlagSlotState;
    readonly maxWidth?: number;
    readonly muted?: boolean;
  },
): { readonly width: number; readonly label: FittedLabel } {
  const swatchW = options?.dash?.length ? 14 : options?.hatch ? 10 : options?.outlined ? 9 : 12;
  const gap = 6;
  let cursor = x;
  ctx.save();
  if (options?.hatch) {
    drawHatchRect(ctx, cursor, Math.round(middleY - 5), swatchW, 10, 1.5, theme);
    cursor += swatchW + gap;
  } else if (options?.outlined) {
    ctx.strokeStyle = theme.muted;
    ctx.lineWidth = 1;
    ctx.setLineDash([2, 2]);
    ctx.beginPath();
    ctx.roundRect(cursor + 0.5, Math.round(middleY - 4.5) + 0.5, 8, 8, 2);
    ctx.stroke();
    ctx.setLineDash([]);
    cursor += swatchW + gap;
  } else if (options?.dash?.length) {
    ctx.strokeStyle = color;
    ctx.lineWidth = 2;
    ctx.setLineDash([...options.dash]);
    ctx.beginPath();
    ctx.moveTo(cursor, middleY);
    ctx.lineTo(cursor + swatchW, middleY);
    ctx.stroke();
    ctx.setLineDash([]);
    cursor += swatchW + gap;
  } else if (color) {
    ctx.fillStyle = color;
    ctx.beginPath();
    ctx.roundRect(cursor, Math.round(middleY - 1.5), swatchW, 3, 1.5);
    ctx.fill();
    cursor += swatchW + gap;
  }
  if (options?.flag && options.flag.kind !== 'pending') {
    const fw = 16;
    const fh = 10;
    drawFlagSlot(ctx, cursor, Math.round(middleY - fh / 2), options.flag, theme, { width: fw, height: fh });
    cursor += fw + gap;
  }
  const fontSize = theme.fontSize.small;
  ctx.font = `${LABEL_WEIGHT} ${fontSize}px ${theme.font}`;
  ctx.fillStyle = options?.muted ? theme.muted : theme.text;
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  const usedW = cursor - x;
  const textMax = options?.maxWidth !== undefined ? Math.max(0, options.maxWidth - usedW) : 10_000;
  const fitted = drawFittedLabel(ctx, label, cursor, middleY, textMax, {
    fontSize,
    lineHeight: labelLineHeight(fontSize),
    maxLines: 1,
  });
  const drawnW = fitted.lines[0] ? textWidth(ctx, fitted.lines[0]) : 0;
  ctx.restore();
  return { width: usedW + drawnW, label: fitted };
}

export function drawDateColumn(
  ctx: Canvas2D,
  x: number,
  y: number,
  rowHeight: number,
  hour: number | null,
  theme: FamilyTheme,
  options?: { readonly width?: number; readonly align?: 'left' | 'right' },
): { readonly missing: boolean; readonly full: string; readonly hover: string } {
  const width = options?.width ?? DATE_COLUMN_WIDTH;
  const align = options?.align ?? 'left';
  const parts = hour === null ? null : formatDateParts(hour);
  const middleY = y + rowHeight / 2;
  if (!parts) {
    const boxX = align === 'right' ? x + Math.max(0, width - MISSING_BOX_SIZE) : x;
    drawHatchRect(ctx, boxX, Math.round(middleY - MISSING_BOX_SIZE / 2), MISSING_BOX_SIZE, MISSING_BOX_SIZE, 2, theme);
    return { missing: true, full: notRecordedLabel('Date'), hover: notRecordedLabel('Date') };
  }
  const anchorX = align === 'right' ? x + width : x;
  ctx.save();
  ctx.textAlign = align;
  ctx.textBaseline = 'middle';
  if (rowHeight >= 32) {
    ctx.font = `500 12.5px ${theme.font}`;
    ctx.fillStyle = theme.text;
    ctx.fillText(ellipsise(parts.dayMonth, width, s => textWidth(ctx, s)), anchorX, middleY - 6);
    ctx.font = `400 10.5px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.fillText(ellipsise(parts.year, width, s => textWidth(ctx, s)), anchorX, middleY + 7);
  } else {
    ctx.font = `500 11.5px ${theme.font}`;
    ctx.fillStyle = theme.text;
    ctx.fillText(ellipsise(parts.dayMonth, width, s => textWidth(ctx, s)), anchorX, middleY);
  }
  ctx.restore();
  return { missing: false, full: parts.full, hover: parts.hover };
}

export function drawMissingMark(
  ctx: Canvas2D,
  x: number,
  middleY: number,
  maxWidth: number,
  theme: FamilyTheme,
  options?: { readonly figure?: string; readonly gameWindow?: boolean; readonly align?: 'left' | 'right'; readonly notApplicable?: boolean },
): { readonly width: number; readonly squareOnly: boolean; readonly label: string } {
  const align = options?.align ?? 'left';
  const smallFont = `${LABEL_WEIGHT} ${theme.fontSize.small}px ${theme.font}`;
  if (options?.notApplicable) {
    ctx.save();
    ctx.font = `${VALUE_WEIGHT} ${theme.fontSize.body}px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.textAlign = align;
    ctx.textBaseline = 'middle';
    ctx.fillText(NOT_APPLICABLE_LABEL, align === 'right' ? x + maxWidth : x, middleY);
    const w = textWidth(ctx, NOT_APPLICABLE_LABEL);
    ctx.restore();
    return { width: w, squareOnly: false, label: NOT_APPLICABLE_LABEL };
  }
  if (options?.gameWindow) {
    ctx.save();
    ctx.font = smallFont;
    ctx.fillStyle = theme.muted;
    ctx.textAlign = align;
    ctx.textBaseline = 'middle';
    const shown = ellipsise(GAME_WINDOW_MISSING_LABEL, maxWidth, s => textWidth(ctx, s));
    ctx.fillText(shown, align === 'right' ? x + maxWidth : x, middleY);
    const w = textWidth(ctx, shown);
    ctx.restore();
    return { width: w, squareOnly: false, label: GAME_WINDOW_MISSING_LABEL };
  }
  const label = notRecordedLabel(options?.figure);
  ctx.save();
  ctx.font = smallFont;
  const labelW = textWidth(ctx, label);
  const fullW = MISSING_BOX_SIZE + MISSING_BOX_GAP + labelW;
  const showWords = maxWidth >= fullW;
  const totalW = showWords ? fullW : MISSING_BOX_SIZE;
  const boxX = align === 'right' ? Math.max(x, x + maxWidth - totalW) : x;
  const boxY = Math.round(middleY - MISSING_BOX_SIZE / 2);
  drawHatchRect(ctx, boxX, boxY, MISSING_BOX_SIZE, MISSING_BOX_SIZE, 2, theme);
  if (showWords) {
    ctx.fillStyle = theme.muted;
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    ctx.fillText(label, boxX + MISSING_BOX_SIZE + MISSING_BOX_GAP, middleY);
  }
  ctx.restore();
  return { width: totalW, squareOnly: !showWords, label };
}

export function drawPartialTab(
  ctx: Canvas2D,
  x: number,
  middleY: number,
  theme: FamilyTheme,
): { readonly width: number; readonly height: number } {
  drawHatchRect(ctx, x, Math.round(middleY - PARTIAL_TAB_HEIGHT / 2), PARTIAL_TAB_WIDTH, PARTIAL_TAB_HEIGHT, 1.5, theme);
  return { width: PARTIAL_TAB_WIDTH, height: PARTIAL_TAB_HEIGHT };
}

export function drawGapBand(
  ctx: Canvas2D,
  x: number,
  y: number,
  width: number,
  height: number,
  theme: FamilyTheme,
): void {
  if (width <= 0 || height <= 0) return;
  const tokens = resolveKitTokens(theme);
  ctx.save();
  ctx.fillStyle = tokens.headerBand;
  ctx.fillRect(x, y, width, height);
  drawHatchRect(ctx, x, y, width, height, 0, theme);
  ctx.restore();
}

export function drawSeriesMissingStrip(
  ctx: Canvas2D,
  x: number,
  y: number,
  width: number,
  color: string,
  theme: FamilyTheme,
  height = SERIES_MISSING_STRIP_HEIGHT,
): void {
  if (width <= 0 || height <= 0) return;
  ctx.save();
  ctx.fillStyle = rgbaColor(color, 0.16, color);
  ctx.fillRect(x, y, width, height);
  drawHatchRect(ctx, x, y, width, height, 0, {
    hatch: color,
    hatchEdge: color,
    line: theme.line,
    hatchSpacing: Math.max(4, Math.round(theme.hatchSpacing * 0.75)),
  });
  ctx.restore();
}

export function drawPlayheadLine(
  ctx: Canvas2D,
  x: number,
  top: number,
  bottom: number,
  theme: FamilyTheme,
): void {
  if (bottom <= top) return;
  const tokens = resolveKitTokens(theme);
  const px = Math.floor(x) + 0.5;
  ctx.save();
  ctx.strokeStyle = tokens.playhead;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(px, top);
  ctx.lineTo(px, bottom);
  ctx.stroke();
  ctx.restore();
}

export function drawCrosshairLine(
  ctx: Canvas2D,
  x: number,
  top: number,
  bottom: number,
  theme: FamilyTheme,
  dash?: readonly number[],
): void {
  if (bottom <= top) return;
  const tokens = resolveKitTokens(theme);
  const px = Math.floor(x) + 0.5;
  ctx.save();
  ctx.strokeStyle = tokens.crosshair;
  ctx.lineWidth = 1;
  if (dash?.length) ctx.setLineDash([...dash]);
  ctx.beginPath();
  ctx.moveTo(px, top);
  ctx.lineTo(px, bottom);
  ctx.stroke();
  if (dash?.length) ctx.setLineDash([]);
  ctx.restore();
}

export function drawDateTab(
  ctx: Canvas2D,
  text: string,
  cx: number,
  y: number,
  theme: FamilyTheme,
  variant: 'playhead' | 'hover' = 'playhead',
  bounds?: { readonly left: number; readonly right: number },
): { readonly x: number; readonly y: number; readonly width: number; readonly height: number } {
  const tokens = resolveKitTokens(theme);
  const padX = 7;
  const height = 19;
  ctx.save();
  ctx.font = `${VALUE_WEIGHT} ${theme.fontSize.small}px ${theme.font}`;
  const textW = textWidth(ctx, text);
  const width = Math.ceil(textW + padX * 2);
  let x = Math.round(cx - width / 2);
  if (bounds) {
    x = Math.max(bounds.left, Math.min(x, bounds.right - width));
  }
  ctx.fillStyle = variant === 'playhead' ? tokens.dateTabBg : tokens.hoverTabBg;
  ctx.beginPath();
  ctx.roundRect(x, y, width, height, 4);
  ctx.fill();
  if (variant === 'playhead') {
    ctx.strokeStyle = tokens.bandDivider;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.roundRect(x + 0.5, y + 0.5, Math.max(0, width - 1), Math.max(0, height - 1), 4);
    ctx.stroke();
  }
  ctx.fillStyle = variant === 'playhead' ? tokens.dateTabText : tokens.hoverTabText;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  ctx.fillText(text, x + width / 2, y + height / 2 + 0.5);
  ctx.restore();
  return { x, y, width, height };
}

export function contrastInkForFill(fill: string, theme: FamilyTheme): string {
  const rgb = parseColor(fill);
  if (!rgb) return theme.text;
  return luminance(rgb) > 0.32 ? 'rgba(17, 17, 17, 0.88)' : 'rgba(255, 255, 255, 0.96)';
}

export function drawChevron(
  ctx: Canvas2D,
  x: number,
  middleY: number,
  open: boolean,
  theme: FamilyTheme,
): void {
  const cx = Math.round(x + 8);
  const cy = Math.round(middleY);
  ctx.save();
  ctx.strokeStyle = theme.muted;
  ctx.lineWidth = 1.5;
  ctx.lineCap = 'round';
  ctx.lineJoin = 'round';
  ctx.beginPath();
  if (open) {
    ctx.moveTo(cx - 3.5, cy - 2);
    ctx.lineTo(cx, cy + 2);
    ctx.lineTo(cx + 3.5, cy - 2);
  } else {
    ctx.moveTo(cx - 2, cy - 3.5);
    ctx.lineTo(cx + 2, cy);
    ctx.lineTo(cx - 2, cy + 3.5);
  }
  ctx.stroke();
  ctx.restore();
}

export const TIMELINE_NOTCH_WIDTH = 9;
export const TIMELINE_FADE_WIDTH = 12;
export const TIMELINE_BAR_RADIUS = 2.5;
export const TIMELINE_LABEL_MIN_WIDTH = 22;

export interface TimelineBarOptions {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
  readonly fill: string;
  readonly theme: FamilyTheme;
  readonly open?: boolean;
  readonly heldBefore?: boolean;
  readonly startBy?: boolean;
  readonly endUnseen?: boolean;
  readonly icon?: ArtSlotState;
  readonly label?: string;
}

export interface TimelineBarResult {
  readonly showedIcon: boolean;
  readonly label: FittedLine | null;
}

function fadeFill(ctx: Canvas2D, x: number, width: number, fill: string, fadeStart: boolean, fadeEnd: boolean): CanvasGradient | string {
  if ((!fadeStart && !fadeEnd) || width <= 2) return fill;
  const grad = ctx.createLinearGradient(x, 0, x + width, 0);
  const stop = Math.min(0.45, TIMELINE_FADE_WIDTH / width);
  const clear = rgbaColor(fill, 0, 'rgba(0,0,0,0)');
  grad.addColorStop(0, fadeStart ? clear : fill);
  if (fadeStart) grad.addColorStop(stop, fill);
  if (fadeEnd) grad.addColorStop(1 - stop, fill);
  grad.addColorStop(1, fadeEnd ? clear : fill);
  return grad;
}

export function drawTimelineBar(ctx: Canvas2D, options: TimelineBarOptions): TimelineBarResult {
  const { x, y, width, height, theme } = options;
  if (width <= 0 || height <= 0) return { showedIcon: false, label: null };
  const fill = options.open ? options.fill : mix(options.fill, theme.surface, 0.5);
  const notch = options.heldBefore && width > TIMELINE_NOTCH_WIDTH + 4 ? TIMELINE_NOTCH_WIDTH : 0;
  const radius = Math.min(TIMELINE_BAR_RADIUS, width / 2, height / 2);
  const left = notch > 0 || options.startBy ? 0 : radius;
  const right = options.open || options.endUnseen ? 0 : radius;
  ctx.save();
  ctx.fillStyle = fadeFill(ctx, x, width, fill, !!options.startBy && notch === 0, !!options.endUnseen);
  ctx.beginPath();
  ctx.roundRect(x, y, width, height, [left, right, right, left]);
  ctx.fill();
  if (notch > 0) {
    const cy = y + height / 2;
    const arm = Math.min(3.5, height * 0.22);
    ctx.strokeStyle = contrastInkForFill(fill, theme);
    ctx.lineWidth = 1.5;
    ctx.lineCap = 'round';
    ctx.lineJoin = 'round';
    ctx.beginPath();
    ctx.moveTo(x + 3 + arm * 0.8, cy - arm);
    ctx.lineTo(x + 3, cy);
    ctx.lineTo(x + 3 + arm * 0.8, cy + arm);
    ctx.stroke();
  }
  let showedIcon = false;
  let label: FittedLine | null = null;
  if (width >= TIMELINE_LABEL_MIN_WIDTH) {
    let cursor = x + (notch > 0 ? notch + 2 : 4);
    const limit = x + width - 4;
    if (options.icon && options.icon.kind !== 'none') {
      const size = options.icon.kind === 'missing' ? Math.min(14, height) : height;
      if (cursor + size <= limit) {
        drawArtSlot(ctx, cursor, y + (height - size) / 2, size, size, options.icon, theme, { radius: 2 });
        cursor += size + 6;
        showedIcon = true;
      }
    }
    if (options.label && limit - cursor >= 12) {
      ctx.font = `500 11.5px ${theme.font}`;
      const fitted = fitLine(options.label, limit - cursor, s => textWidth(ctx, s));
      if (fitted.text) {
        ctx.fillStyle = contrastInkForFill(fill, theme);
        ctx.textAlign = 'left';
        ctx.textBaseline = 'middle';
        ctx.fillText(fitted.text, cursor, y + height / 2 + 0.5);
        label = fitted;
      }
    }
  }
  ctx.restore();
  return { showedIcon, label };
}

export interface TimeAxisTick {
  readonly hour: number;
  readonly label: string;
  readonly major?: boolean;
}

export function computeTimeAxisTicks(startHour: number, endHour: number, plotWidth: number, dayHours = 24): TimeAxisTick[] {
  if (plotWidth <= 20 || endHour <= startHour) return [];
  const hoursPerDay = Math.max(1, dayHours);
  const spanHours = endHour - startHour;
  const start = gameDate(Math.floor(startHour));
  const end = gameDate(Math.floor(endHour));
  const ticks: TimeAxisTick[] = [];
  if (!start || !end || spanHours <= 62 * hoursPerDay) {
    const spanDays = Math.max(1, (spanHours + 1) / hoursPerDay);
    const fit = Math.max(2, Math.floor(plotWidth / 64));
    const step = [1, 2, 3, 5, 7, 14].find(candidate => Math.ceil(spanDays / candidate) <= fit) ?? 14;
    const firstDay = Math.floor((startHour - HOUR_EPOCH) / hoursPerDay);
    const lastDay = Math.floor((endHour - HOUR_EPOCH) / hoursPerDay);
    for (let day = firstDay; day <= lastDay; day++) {
      if (((day % step) + step) % step !== 0) continue;
      const hour = HOUR_EPOCH + day * hoursPerDay;
      if (hour <= startHour || hour > endHour) continue;
      const parts = formatDateParts(hour);
      const date = gameDate(hour);
      ticks.push({ hour, label: parts?.dayMonth ?? formatDateOnly(hour), major: date ? date.day === 1 : false });
    }
    return ticks;
  }
  if (spanHours < 2 * 365 * hoursPerDay) {
    const months = (end.year - start.year) * 12 + (end.month - start.month) + 1;
    const fit = Math.max(2, Math.floor(plotWidth / 56));
    const step = [1, 2, 3, 6, 12].find(candidate => Math.ceil(months / candidate) <= fit) ?? 12;
    let year = start.year;
    let month = start.month;
    if (start.day > 15) {
      month++;
      if (month > 12) { month = 1; year++; }
    }
    while (year < end.year || (year === end.year && month <= end.month)) {
      if ((month - 1) % step === 0) {
        const rawHour = engineHour({ year, month, day: 1, hour: 1 });
        const hour = rawHour !== null && rawHour < startHour && year === start.year && month === start.month && start.day <= 15
          ? startHour
          : rawHour;
        if (hour !== null && hour >= startHour && hour <= endHour) {
          const major = month === 1;
          ticks.push({ hour, label: major ? String(year) : monthNames[month - 1]!.slice(0, 3), major });
        }
      }
      month++;
      if (month > 12) { month = 1; year++; }
    }
    return ticks;
  }
  const years = end.year - start.year + 1;
  const fit = Math.max(2, Math.floor(plotWidth / 48));
  const step = [1, 2, 5, 10, 20, 50].find(candidate => Math.ceil(years / candidate) <= fit) ?? 100;
  for (let year = start.year; year <= end.year; year++) {
    if (year % step !== 0) continue;
    const rawHour = engineHour({ year, month: 1, day: 1, hour: 1 });
    const hour = rawHour !== null && rawHour < startHour && year === start.year && start.month === 1 && start.day <= 15
      ? startHour
      : rawHour;
    if (hour !== null && hour >= startHour && hour <= endHour) ticks.push({ hour, label: String(year), major: true });
  }
  return ticks;
}

export interface TimeAxisFrame {
  readonly plotLeft: number;
  readonly plotRight: number;
  readonly startHour: number;
  readonly endHour: number;
}

export function timeAxisX(frame: TimeAxisFrame, hour: number): number {
  const span = Math.max(1, frame.endHour - frame.startHour);
  return frame.plotLeft + ((hour - frame.startHour) / span) * (frame.plotRight - frame.plotLeft);
}

export function drawTimeAxisGrid(
  ctx: Canvas2D,
  ticks: readonly TimeAxisTick[],
  frame: TimeAxisFrame,
  top: number,
  bottom: number,
  theme: FamilyTheme,
): void {
  const tokens = resolveKitTokens(theme);
  ctx.save();
  ctx.strokeStyle = tokens.hairline;
  ctx.lineWidth = 1;
  ctx.beginPath();
  for (const tick of ticks) {
    const x = Math.floor(timeAxisX(frame, tick.hour)) + 0.5;
    if (x <= frame.plotLeft + 1 || x >= frame.plotRight - 1) continue;
    ctx.moveTo(x, top);
    ctx.lineTo(x, bottom);
  }
  ctx.stroke();
  ctx.restore();
}

export interface TimeAxisOptions extends TimeAxisFrame {
  readonly axisTop: number;
  readonly playhead: number | null;
  readonly playheadLabel: string;
  readonly hover?: number | null;
  readonly hoverLabel?: string;
  readonly playheadLineTop?: number | null;
  readonly hairline?: 'normal' | 'strong' | 'band' | 'none';
  readonly tickMarks?: boolean;
  readonly tabBounds?: { readonly left: number; readonly right: number };
  readonly tabY?: number;
  readonly labelY?: number;
  readonly theme: FamilyTheme;
}

export function drawTimeAxis(ctx: Canvas2D, ticks: readonly TimeAxisTick[], options: TimeAxisOptions): void {
  const { plotLeft, plotRight, axisTop, theme } = options;
  const tokens = resolveKitTokens(theme);
  const hairline = options.hairline ?? 'band';
  if (hairline !== 'none') drawRowHairline(ctx, plotLeft, plotRight, axisTop, theme, hairline);
  const tabBounds = options.tabBounds ?? { left: plotLeft, right: plotRight };
  const tabY = options.tabY ?? axisTop + 4;
  const labelY = options.labelY ?? axisTop + 14;
  const showTicks = options.tickMarks !== false;
  ctx.save();
  ctx.font = `400 ${theme.fontSize.small}px ${theme.font}`;
  const keepOut: [number, number][] = [];
  let px: number | null = null;
  if (options.playhead !== null) {
    px = timeAxisX(options, options.playhead);
    const tabW = textWidth(ctx, options.playheadLabel) + 14;
    const left = Math.max(tabBounds.left, Math.min(tabBounds.right - tabW, Math.round(px - tabW / 2)));
    keepOut.push([left - 6, left + tabW + 6]);
  }
  let hx: number | null = null;
  if (options.hover != null && options.hoverLabel) {
    hx = timeAxisX(options, options.hover);
    const tabW = textWidth(ctx, options.hoverLabel) + 14;
    const left = Math.max(tabBounds.left, Math.min(tabBounds.right - tabW, Math.round(hx - tabW / 2)));
    keepOut.push([left - 6, left + tabW + 6]);
  }
  ctx.fillStyle = theme.muted;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  ctx.strokeStyle = tokens.hairlineStrong;
  ctx.lineWidth = 1;
  let lastRight = -Infinity;
  for (const tick of ticks) {
    const x = timeAxisX(options, tick.hour);
    const half = textWidth(ctx, tick.label) / 2;
    if (showTicks) {
      ctx.beginPath();
      ctx.moveTo(Math.floor(x) + 0.5, axisTop);
      ctx.lineTo(Math.floor(x) + 0.5, axisTop + (tick.major === false ? 3 : 4));
      ctx.stroke();
    }
    if (x - half < plotLeft - 4 || x + half > plotRight + 4) continue;
    if (keepOut.some(([l, r]) => x + half > l && x - half < r)) continue;
    if (x - half < lastRight + 8) continue;
    ctx.fillText(tick.label, x, labelY);
    lastRight = x + half;
  }
  ctx.restore();
  if (px !== null) {
    if (options.playheadLineTop !== null) {
      drawPlayheadLine(ctx, px, options.playheadLineTop ?? 0, axisTop, theme);
    }
    drawDateTab(ctx, options.playheadLabel, px, tabY, theme, 'playhead', tabBounds);
  }
  if (hx !== null && options.hoverLabel) {
    drawDateTab(ctx, options.hoverLabel, hx, tabY, theme, 'hover', tabBounds);
  }
}

export function winRateColor(rate: number, theme?: FamilyTheme): string {
  const dark = theme ? isDarkTheme(theme) : true;
  const low: readonly [number, number, number] = dark ? [255, 107, 111] : [207, 46, 54];
  const mid: readonly [number, number, number] = dark ? [212, 212, 212] : [92, 100, 112];
  const high: readonly [number, number, number] = dark ? [111, 212, 150] : [28, 138, 76];
  const mix = (a: readonly [number, number, number], b: readonly [number, number, number], t: number) =>
    `rgb(${Math.round(a[0] + (b[0] - a[0]) * t)}, ${Math.round(a[1] + (b[1] - a[1]) * t)}, ${Math.round(a[2] + (b[2] - a[2]) * t)})`;
  if (rate <= 0.35) return mix(low, low, 0);
  if (rate >= 0.65) return mix(high, high, 0);
  if (rate < 0.5) return mix(low, mid, (rate - 0.35) / 0.15);
  return mix(mid, high, (rate - 0.5) / 0.15);
}

export function drawWinRateCell(
  ctx: Canvas2D,
  left: number,
  right: number,
  middleY: number,
  won: number | null,
  lost: number | null,
  theme: FamilyTheme,
): { readonly pct: number | null } {
  const total = won !== null && lost !== null ? won + lost : 0;
  ctx.save();
  ctx.textBaseline = 'middle';
  if (total <= 0) {
    ctx.font = `${LABEL_WEIGHT} 13.5px ${theme.font}`;
    ctx.fillStyle = theme.muted;
    ctx.textAlign = 'right';
    ctx.fillText(NOT_APPLICABLE_LABEL, right, middleY);
    ctx.restore();
    return { pct: null };
  }
  const rate = Math.max(0, Math.min(1, won! / total));
  const pct = Math.round(rate * 100);
  ctx.font = `600 13.5px ${theme.font}`;
  ctx.fillStyle = winRateColor(rate, theme);
  ctx.textAlign = 'right';
  ctx.fillText(`${pct}%`, right, middleY);
  const barW = 40;
  const barH = 4;
  const pctSlot = 38;
  const gap = 9;
  const barX = right - pctSlot - gap - barW;
  if (barX >= left) {
    const barY = Math.round(middleY - barH / 2);
    const hasBoth = won! > 0 && lost! > 0;
    const splitGap = hasBoth ? 1 : 0;
    const usable = barW - splitGap;
    const wonW = won! > 0 ? Math.max(2, Math.round(usable * rate)) : 0;
    const lostW = lost! > 0 ? Math.max(2, usable - wonW) : 0;
    if (wonW > 0) {
      ctx.fillStyle = '#4fbf7a';
      ctx.beginPath();
      ctx.roundRect(barX, barY, wonW, barH, [2, hasBoth ? 0 : 2, hasBoth ? 0 : 2, 2]);
      ctx.fill();
    }
    if (lostW > 0) {
      ctx.fillStyle = '#e5484d';
      ctx.beginPath();
      ctx.roundRect(barX + wonW + splitGap, barY, lostW, barH, [hasBoth ? 0 : 2, 2, 2, hasBoth ? 0 : 2]);
      ctx.fill();
    }
  }
  ctx.restore();
  return { pct };
}

export function drawOutOfFigure(
  ctx: Canvas2D,
  valueText: string,
  maxText: string,
  left: number,
  right: number,
  middleY: number,
  theme: FamilyTheme,
  options?: { readonly muted?: boolean; readonly fontSize?: number },
): void {
  const size = options?.fontSize ?? 13.5;
  const suffix = ` / ${maxText}`;
  const valFont = `500 ${size}px ${theme.font}`;
  const sufFont = `400 ${Math.max(11, size - 1.5)}px ${theme.font}`;
  ctx.save();
  ctx.textBaseline = 'middle';
  ctx.textAlign = 'right';
  ctx.font = sufFont;
  const sufW = textWidth(ctx, suffix);
  const avail = Math.max(0, right - left);
  if (sufW < avail) {
    ctx.fillStyle = theme.muted;
    ctx.fillText(suffix, right, middleY);
    ctx.font = valFont;
    ctx.fillStyle = options?.muted ? theme.muted : theme.text;
    ctx.fillText(ellipsise(valueText, Math.max(0, avail - sufW), s => textWidth(ctx, s)), right - sufW, middleY);
  } else {
    ctx.font = valFont;
    ctx.fillStyle = options?.muted ? theme.muted : theme.text;
    ctx.fillText(ellipsise(`${valueText}${suffix}`, avail, s => textWidth(ctx, s)), right, middleY);
  }
  ctx.restore();
}

export function drawSkillFigure(
  ctx: Canvas2D,
  left: number,
  right: number,
  middleY: number,
  valueText: string,
  iconState: ArtSlotState,
  theme: FamilyTheme,
  options?: { readonly badge?: boolean; readonly muted?: boolean },
): void {
  const iconW = options?.badge ? 16 : 18;
  const iconH = options?.badge ? 23 : 18;
  const gap = 7;
  const font = `${options?.badge ? 600 : 500} 13.5px ${theme.font}`;
  ctx.save();
  ctx.font = font;
  ctx.textAlign = 'right';
  ctx.textBaseline = 'middle';
  ctx.fillStyle = options?.muted ? theme.muted : theme.text;
  const numW = Math.max(14, textWidth(ctx, valueText));
  ctx.fillText(valueText, right, middleY);
  const iconX = right - numW - gap - iconW;
  if (iconX >= left) {
    drawArtSlot(ctx, iconX, Math.round(middleY - iconH / 2), iconW, iconH, iconState, theme, { radius: 2 });
  }
  ctx.restore();
}

export interface IconStripItem {
  readonly state: ArtSlotState;
  readonly name: string;
  readonly count?: number;
}

export function drawIconStrip(
  ctx: Canvas2D,
  left: number,
  right: number,
  middleY: number,
  items: readonly IconStripItem[],
  theme: FamilyTheme,
  options?: { readonly kind?: 'traits' | 'medals' },
): { readonly drawnCount: number; readonly overflowCount: number } {
  const size = options?.kind === 'medals' ? MEDAL_ICON_SIZE : TRAIT_ICON_SIZE;
  const gap = ICON_STRIP_GAP;
  const y = Math.round(middleY - size / 2);
  let cursor = left;
  let drawnCount = 0;
  for (let i = 0; i < items.length; i++) {
    const item = items[i]!;
    const countText = item.count && item.count > 1 ? `\u00d7${item.count}` : '';
    let countW = 0;
    if (countText) {
      ctx.save();
      ctx.font = `600 11px ${theme.font}`;
      countW = textWidth(ctx, countText) + 3;
      ctx.restore();
    }
    if (cursor + size + countW > right) break;
    drawArtSlot(ctx, cursor, y, size, size, item.state, theme, { radius: 3 });
    cursor += size;
    if (countText) {
      ctx.save();
      ctx.font = `600 11px ${theme.font}`;
      ctx.fillStyle = theme.text;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillText(countText, cursor + 2, middleY);
      ctx.restore();
      cursor += countW;
    }
    cursor += gap;
    drawnCount++;
  }
  return { drawnCount, overflowCount: items.length - drawnCount };
}

export function drawActionButton(
  ctx: Canvas2D,
  right: number,
  middleY: number,
  label: string,
  active: boolean,
  theme: FamilyTheme,
): { readonly x: number; readonly y: number; readonly width: number; readonly height: number } {
  const tokens = resolveKitTokens(theme);
  const iconSize = 13;
  const drawWindowIcon = (ix: number, iy: number, color: string, alpha: number) => {
    ctx.save();
    ctx.globalAlpha = alpha;
    ctx.strokeStyle = color;
    ctx.lineWidth = 1.3;
    ctx.beginPath();
    ctx.roundRect(ix + 0.75, iy + 1.25, iconSize - 1.5, iconSize - 2.5, 2);
    ctx.stroke();
    ctx.beginPath();
    ctx.moveTo(ix + 1, iy + 4.5);
    ctx.lineTo(ix + iconSize - 1, iy + 4.5);
    ctx.stroke();
    ctx.restore();
  };
  if (!active) {
    const ix = right - iconSize;
    const iy = Math.round(middleY - iconSize / 2);
    drawWindowIcon(ix, iy, theme.muted, 0.55);
    return { x: ix - 4, y: iy - 4, width: iconSize + 8, height: iconSize + 8 };
  }
  const font = `500 12px ${theme.font}`;
  ctx.save();
  ctx.font = font;
  const labelW = textWidth(ctx, label);
  const padX = 8;
  const gap = 5;
  const h = 24;
  const w = Math.ceil(padX * 2 + iconSize + gap + labelW);
  const x = right - w;
  const y = Math.round(middleY - h / 2);
  ctx.fillStyle = tokens.dark ? 'rgba(255, 255, 255, 0.09)' : 'rgba(15, 23, 42, 0.08)';
  ctx.beginPath();
  ctx.roundRect(x, y, w, h, ROW_CORNER_RADIUS);
  ctx.fill();
  drawWindowIcon(x + padX, Math.round(middleY - iconSize / 2), theme.text, 0.9);
  ctx.fillStyle = theme.text;
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  ctx.fillText(label, x + padX + iconSize + gap, middleY);
  ctx.restore();
  return { x, y, width: w, height: h };
}

export function drawStockpileSumCell(
  ctx: Canvas2D,
  x: number,
  width: number,
  rowY: number,
  rowHeight: number,
  balance: number | null,
  maxAbs: number,
  labelText: string,
  theme: FamilyTheme,
  options?: { readonly missing?: boolean },
): void {
  const tokens = resolveKitTokens(theme);
  const middleY = rowY + rowHeight / 2;
  const figSlot = 68;
  const barRight = Math.max(x + 24, x + width - figSlot - 8);
  const barAreaW = Math.max(0, barRight - x);
  const centerX = Math.floor(x + barAreaW / 2) + 0.5;

  ctx.save();
  ctx.strokeStyle = tokens.dark ? 'rgba(255, 255, 255, 0.14)' : 'rgba(15, 23, 42, 0.16)';
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(centerX, rowY);
  ctx.lineTo(centerX, rowY + rowHeight);
  ctx.stroke();
  ctx.restore();

  if (options?.missing) {
    drawMissingMark(ctx, x + width - figSlot, middleY, figSlot, theme, { align: 'right' });
    return;
  }

  if (balance !== null && balance !== 0 && maxAbs > 0 && barAreaW >= 16) {
    const halfW = Math.max(0, barAreaW / 2 - 4);
    const frac = Math.min(1, Math.abs(balance) / maxAbs);
    const barW = Math.min(halfW, Math.max(4, Math.round(halfW * frac)));
    const barH = 16;
    const barY = Math.round(middleY - barH / 2);
    if (balance < 0) {
      drawBar(ctx, centerX - barW, barY, barW, barH, tokens.shortage, 'left', 0.85);
    } else {
      drawBar(ctx, centerX, barY, barW, barH, tokens.dark ? 'rgba(255, 255, 255, 0.28)' : 'rgba(15, 23, 42, 0.24)', 'right', 1);
    }
  }

  ctx.save();
  ctx.font = `500 13.5px ${theme.font}`;
  ctx.textAlign = 'right';
  ctx.textBaseline = 'middle';
  ctx.fillStyle = balance === null || balance === 0
    ? theme.muted
    : balance < 0
      ? tokens.shortage
      : theme.text;
  ctx.fillText(labelText, x + width, middleY);
  ctx.restore();
}

export interface ClockSummaryData {
  readonly totalText: string;
  readonly playingText: string;
  readonly pausedText: string;
  readonly playingFraction: number | null;
  readonly pausedFraction: number | null;
  readonly playingPct: string | null;
  readonly pausedPct: string | null;
  readonly complete: boolean;
  readonly coverageLabel: string;
  readonly spanText: string | null;
}

export function drawClockSummary(
  ctx: Canvas2D,
  width: number,
  height: number,
  summary: ClockSummaryData,
  theme: FamilyTheme,
): void {
  const tokens = resolveKitTokens(theme);
  const padX = 22;
  const innerW = Math.max(1, width - padX * 2);
  ctx.save();
  ctx.fillStyle = theme.surface;
  ctx.fillRect(0, 0, width, height);
  ctx.textAlign = 'left';

  const figureFont = `600 36px ${theme.font}`;
  const captionFont = `400 13px ${theme.font}`;
  const labelFont = `400 13px ${theme.font}`;
  const timeFont = `600 13px ${theme.font}`;
  const secondary = tokens.dark ? '#d4d4d4' : theme.text;

  ctx.textBaseline = 'alphabetic';
  ctx.font = figureFont;
  ctx.fillStyle = theme.text;
  const figure = ellipsise(summary.totalText, innerW, s => textWidth(ctx, s));
  ctx.fillText(figure, padX, 57);
  const figureW = textWidth(ctx, figure);
  ctx.font = captionFont;
  ctx.fillStyle = theme.muted;
  const captionX = padX + figureW + 12;
  if (captionX < padX + innerW) {
    ctx.fillText(ellipsise('Recorded, in real time', padX + innerW - captionX, s => textWidth(ctx, s)), captionX, 57);
  }

  const barY = 83;
  const barH = 8;
  const playColor = tokens.dark ? '#c8c8c8' : '#475569';
  const pauseColor = tokens.dark ? 'rgba(245, 245, 245, 0.16)' : 'rgba(15, 23, 42, 0.14)';
  if (summary.playingFraction !== null && summary.pausedFraction !== null) {
    const gap = summary.playingFraction > 0 && summary.pausedFraction > 0 ? 2 : 0;
    const usable = Math.max(0, innerW - gap);
    const playW = summary.playingFraction > 0 ? Math.max(3, Math.round(usable * summary.playingFraction)) : 0;
    const pauseW = summary.pausedFraction > 0 ? Math.max(3, usable - playW) : 0;
    if (playW > 0) {
      ctx.fillStyle = playColor;
      ctx.beginPath();
      ctx.roundRect(padX, barY, playW, barH, [4, gap ? 0 : 4, gap ? 0 : 4, 4]);
      ctx.fill();
    }
    if (pauseW > 0) {
      ctx.fillStyle = pauseColor;
      ctx.beginPath();
      ctx.roundRect(padX + playW + gap, barY, pauseW, barH, [gap ? 0 : 4, 4, 4, gap ? 0 : 4]);
      ctx.fill();
    }
  } else {
    drawHatchRect(ctx, padX, barY, innerW, barH, 4, theme);
  }

  ctx.textBaseline = 'middle';
  const statWidth = (label: string, time: string) => {
    ctx.font = labelFont;
    const lw = textWidth(ctx, label);
    ctx.font = timeFont;
    return { lw, total: lw + 8 + textWidth(ctx, time) };
  };
  const drawStat = (x: number, cy: number, label: string, time: string) => {
    const { lw } = statWidth(label, time);
    ctx.font = labelFont;
    ctx.fillStyle = secondary;
    ctx.fillText(label, x, cy);
    ctx.font = timeFont;
    ctx.fillStyle = theme.text;
    ctx.fillText(time, x + lw + 8, cy);
  };
  const playing = statWidth('Playing', summary.playingText);
  const paused = statWidth('Paused', summary.pausedText);
  let coverageY = 152;
  drawStat(padX, 117, 'Playing', summary.playingText);
  if (playing.total + 24 + paused.total <= innerW) {
    drawStat(padX + innerW - paused.total, 117, 'Paused', summary.pausedText);
  } else {
    drawStat(padX, 141, 'Paused', summary.pausedText);
    coverageY = 176;
  }

  ctx.fillStyle = summary.complete ? theme.muted : theme.amber;
  ctx.beginPath();
  ctx.arc(padX + 3.5, coverageY, 3.5, 0, Math.PI * 2);
  ctx.fill();
  ctx.font = labelFont;
  ctx.fillStyle = secondary;
  ctx.fillText(summary.coverageLabel, padX + 15, coverageY);
  if (summary.spanText) {
    const spanX = padX + 15 + textWidth(ctx, summary.coverageLabel) + 22;
    if (spanX < padX + innerW) {
      ctx.fillStyle = theme.muted;
      ctx.fillText(ellipsise(summary.spanText, padX + innerW - spanX, s => textWidth(ctx, s)), spanX, coverageY);
    }
  }
  ctx.restore();
}

export function drawGradientLegend(
  ctx: Canvas2D,
  x: number,
  middleY: number,
  lowLabel: string,
  highLabel: string,
  stops: readonly string[],
  theme: FamilyTheme,
  options?: { readonly barWidth?: number; readonly barHeight?: number; readonly gap?: number },
): { readonly width: number } {
  const barWidth = options?.barWidth ?? 64;
  const barHeight = options?.barHeight ?? 8;
  const gap = options?.gap ?? 6;
  const fontSize = theme.fontSize.small;
  ctx.save();
  ctx.font = `${LABEL_WEIGHT} ${fontSize}px ${theme.font}`;
  ctx.fillStyle = theme.muted;
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  const lowW = lowLabel ? textWidth(ctx, lowLabel) : 0;
  const highW = highLabel ? textWidth(ctx, highLabel) : 0;
  let cursor = x;
  if (lowLabel) {
    ctx.fillText(lowLabel, cursor, middleY);
    cursor += lowW + gap;
  }
  const barY = Math.round(middleY - barHeight / 2);
  if (stops.length >= 2 && barWidth > 0) {
    const gradient = ctx.createLinearGradient(cursor, 0, cursor + barWidth, 0);
    const last = stops.length - 1;
    for (let i = 0; i < stops.length; i++) {
      gradient.addColorStop(i / last, stops[i]!);
    }
    ctx.fillStyle = gradient;
    ctx.beginPath();
    ctx.roundRect(cursor, barY, barWidth, barHeight, 2);
    ctx.fill();
  } else if (stops.length === 1 && barWidth > 0) {
    ctx.fillStyle = stops[0]!;
    ctx.beginPath();
    ctx.roundRect(cursor, barY, barWidth, barHeight, 2);
    ctx.fill();
  }
  cursor += barWidth;
  if (highLabel) {
    cursor += gap;
    ctx.fillStyle = theme.muted;
    ctx.fillText(highLabel, cursor, middleY);
    cursor += highW;
  }
  ctx.restore();
  return { width: cursor - x };
}

export interface LegendStripGradient {
  readonly lowLabel: string;
  readonly highLabel: string;
  readonly stops: readonly string[];
}

export interface LegendStripEntry {
  readonly label: string;
  readonly color?: string;
  readonly dash?: readonly number[];
  readonly hatch?: boolean;
  readonly outlined?: boolean;
  readonly flag?: FlagSlotState;
  readonly muted?: boolean;
}

export interface LegendStripSpec {
  readonly gradient?: LegendStripGradient;
  readonly entries?: readonly LegendStripEntry[];
  readonly rightNote?: string;
  readonly padX?: number;
  readonly entryGap?: number;
  readonly hairline?: boolean;
}

export function drawLegendStrip(
  ctx: Canvas2D,
  x: number,
  y: number,
  width: number,
  height: number,
  spec: LegendStripSpec,
  theme: FamilyTheme,
): void {
  if (width <= 0 || height <= 0) return;
  const padX = spec.padX ?? 0;
  const entryGap = spec.entryGap ?? 12;
  const middleY = Math.round(y + height / 2);
  let cursor = x + padX;
  const rightLimit = x + width - padX;
  let rightNoteWidth = 0;
  if (spec.rightNote) {
    ctx.save();
    ctx.font = `${LABEL_WEIGHT} ${theme.fontSize.small}px ${theme.font}`;
    rightNoteWidth = Math.min(Math.floor(width * 0.3), Math.ceil(textWidth(ctx, spec.rightNote)));
    ctx.restore();
  }
  const entriesRightLimit = rightNoteWidth > 0 ? Math.max(cursor, rightLimit - rightNoteWidth - entryGap) : rightLimit;
  if (spec.gradient) {
    const drawn = drawGradientLegend(
      ctx,
      cursor,
      middleY,
      spec.gradient.lowLabel,
      spec.gradient.highLabel,
      spec.gradient.stops,
      theme,
    );
    cursor += drawn.width + 14;
  }
  if (spec.entries) {
    for (const entry of spec.entries) {
      const room = entriesRightLimit - cursor;
      if (room < 32) break;
      const drawn = drawLegendEntry(ctx, cursor, middleY, entry.label, entry.color ?? '', theme, {
        ...(entry.dash?.length ? { dash: entry.dash } : {}),
        ...(entry.hatch ? { hatch: true } : {}),
        ...(entry.outlined ? { outlined: true } : {}),
        ...(entry.flag ? { flag: entry.flag } : {}),
        ...(entry.muted ?? (entry.hatch || entry.outlined) ? { muted: true } : {}),
        maxWidth: room,
      });
      cursor += drawn.width + entryGap;
    }
  }
  if (spec.rightNote) {
    const maxW = Math.max(rightNoteWidth, rightLimit - cursor);
    if (maxW >= 24) {
      ctx.save();
      ctx.font = `${LABEL_WEIGHT} ${theme.fontSize.small}px ${theme.font}`;
      ctx.fillStyle = theme.muted;
      ctx.textAlign = 'right';
      ctx.textBaseline = 'middle';
      ctx.fillText(ellipsise(spec.rightNote, maxW, s => textWidth(ctx, s)), rightLimit, middleY);
      ctx.restore();
    }
  }
  if (spec.hairline) {
    drawRowHairline(ctx, x, x + width, y + height - 1, theme, 'normal');
  }
}

export const ROW_SECTION_HEIGHT = 20;
export const ROW_REMOVE_SIZE = 22;
export const ROW_GRIP_WIDTH = 8;
export const ROW_GRIP_HEIGHT = 12;

export function drawRowSectionLabel(ctx: Canvas2D, x: number, y: number, height: number, label: string, theme: FamilyTheme): void {
  ctx.save();
  ctx.font = `${LABEL_WEIGHT} 11px ${theme.font}`;
  ctx.fillStyle = theme.muted;
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  ctx.fillText(label, x, y + height / 2);
  ctx.restore();
}

export function rowGripBox(nameX: number, middleY: number): { readonly x: number; readonly y: number; readonly width: number; readonly height: number } {
  return { x: Math.max(0, nameX - ROW_GRIP_WIDTH - 1), y: Math.round(middleY - ROW_GRIP_HEIGHT / 2), width: ROW_GRIP_WIDTH, height: ROW_GRIP_HEIGHT };
}

export function drawRowGrip(ctx: Canvas2D, nameX: number, middleY: number, theme: FamilyTheme): void {
  const box = rowGripBox(nameX, middleY);
  ctx.save();
  ctx.fillStyle = theme.muted;
  for (let column = 0; column < 2; column++) {
    for (let line = 0; line < 3; line++) {
      ctx.fillRect(box.x + 1 + column * 4, box.y + 1 + line * 4, 2, 2);
    }
  }
  ctx.restore();
}

export function rowRemoveBox(right: number, middleY: number): { readonly x: number; readonly y: number; readonly width: number; readonly height: number } {
  return { x: right - ROW_REMOVE_SIZE, y: Math.round(middleY - ROW_REMOVE_SIZE / 2), width: ROW_REMOVE_SIZE, height: ROW_REMOVE_SIZE };
}

export function drawRowRemoveButton(ctx: Canvas2D, right: number, middleY: number, theme: FamilyTheme): void {
  const box = rowRemoveBox(right, middleY);
  ctx.save();
  ctx.fillStyle = mix(theme.surface, theme.text, 0.08);
  ctx.beginPath();
  ctx.roundRect(box.x, box.y, box.width, box.height, 4);
  ctx.fill();
  ctx.fillStyle = theme.text;
  ctx.font = `500 10px ${theme.font}`;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  ctx.fillText('\u00d7', box.x + box.width / 2, middleY + 0.5);
  ctx.restore();
}

export function drawAddRow(
  ctx: Canvas2D,
  nameX: number,
  middleY: number,
  lineLeft: number,
  lineRight: number,
  label: string,
  theme: FamilyTheme,
): void {
  const tokens = resolveKitTokens(theme);
  const plus = 12;
  const cx = nameX + plus / 2;
  const cy = Math.round(middleY) + 0.5;
  ctx.save();
  ctx.strokeStyle = theme.accent;
  ctx.lineWidth = 1.5;
  ctx.beginPath();
  ctx.moveTo(cx - plus / 2 + 1, cy);
  ctx.lineTo(cx + plus / 2 - 1, cy);
  ctx.moveTo(cx, cy - plus / 2 + 1);
  ctx.lineTo(cx, cy + plus / 2 - 1);
  ctx.stroke();
  ctx.font = `500 12px ${theme.font}`;
  ctx.fillStyle = theme.accent;
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  ctx.fillText(label, nameX + plus + 6, middleY);
  if (lineRight > lineLeft) {
    ctx.strokeStyle = tokens.hairlineStrong;
    ctx.lineWidth = 1;
    ctx.setLineDash([4, 4]);
    ctx.beginPath();
    ctx.moveTo(lineLeft, cy);
    ctx.lineTo(lineRight, cy);
    ctx.stroke();
  }
  ctx.restore();
}
