import { LABEL_WEIGHT, VALUE_WEIGHT } from '../charts/draw.ts';
import {
  drawFittedLabel,
  ellipsise,
  graphemes,
  labelFitsTwoLines,
  labelLineHeight,
  type FittedLabel,
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
  formatDateParts,
  GAME_WINDOW_MISSING_LABEL,
  NOT_APPLICABLE_LABEL,
  notRecordedLabel,
} from '../families/valueFormat.ts';
import { parseColor } from '../shell/appearance/color.ts';
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
export const BEHIND_BAR_OFFSET = 7;
export const ROW_CORNER_RADIUS = 7;
export const FLAG_SLOT_WIDTH = 20;
export const FLAG_SLOT_HEIGHT = 13;
export const FLAG_LOSER_ALPHA = 0.35;
export const DATE_COLUMN_WIDTH = 50;
export const SERIES_MISSING_STRIP_HEIGHT = 5;

const INITIALS_CACHE_MAX = 1024;
const initialsCache = new Map<string, string>();

function isDarkTheme(theme: Pick<FamilyTheme, 'surface' | 'dark'>): boolean {
  if (theme.dark !== undefined) return theme.dark;
  const rgb = parseColor(theme.surface);
  if (!rgb) return true;
  return 0.2126 * rgb.r + 0.7152 * rgb.g + 0.0722 * rgb.b < 128;
}

function rgba(color: string, alpha: number, fallback: string): string {
  const rgb = parseColor(color);
  if (!rgb) return fallback;
  return `rgba(${Math.round(rgb.r)}, ${Math.round(rgb.g)}, ${Math.round(rgb.b)}, ${alpha})`;
}

export function resolveKitTokens(theme: FamilyTheme) {
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
    rowSelected: theme.rowSelected ?? rgba(theme.accent, 0.14, dark ? 'rgba(96, 165, 250, 0.14)' : 'rgba(37, 99, 235, 0.14)'),
    childTint: theme.childTint ?? `rgba(${ink}, ${dark ? 0.018 : 0.025})`,
    shortage: theme.shortage ?? (dark ? '#ff6b6f' : '#e5484d'),
    barMuted: theme.barMuted ?? `rgba(${ink}, ${dark ? 0.085 : 0.095})`,
    barTrack: theme.barTrack ?? `rgba(${ink}, ${dark ? 0.08 : 0.1})`,
    gridLine: theme.gridLine ?? `rgba(${ink}, ${dark ? 0.055 : 0.07})`,
    zeroLine: theme.zeroLine ?? `rgba(${ink}, ${dark ? 0.18 : 0.22})`,
    tailLine: theme.tailLine ?? rgba(theme.muted, 0.22, `rgba(${ink}, 0.22)`),
    playhead: theme.playhead ?? `rgba(${ink}, ${dark ? 0.28 : 0.34})`,
    crosshair: theme.crosshair ?? `rgba(${ink}, ${dark ? 0.5 : 0.52})`,
    dateTabBg: theme.dateTabBg ?? (theme.raised ?? theme.surface),
    dateTabText: theme.dateTabText ?? theme.text,
    hoverTabBg: theme.hoverTabBg ?? theme.text,
    hoverTabText: theme.hoverTabText ?? theme.surface,
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
  gradient.addColorStop(1, rgba(theme.surface, 0, 'rgba(0, 0, 0, 0)'));
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
  | { readonly kind: 'pending' }
  | { readonly kind: 'ready'; readonly bitmap: CanvasImageSource; readonly width: number; readonly height: number }
  | { readonly kind: 'missing'; readonly tag?: string }
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
  if (width <= 0 || height <= 0 || state.kind === 'pending') {
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
  return { x, y, width, height, drawn: 'failed' };
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
  const width = maxWidth * clamped;
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
    readonly flag?: FlagSlotState;
    readonly maxWidth?: number;
    readonly muted?: boolean;
  },
): { readonly width: number; readonly label: FittedLabel } {
  const swatchW = options?.dash?.length ? 14 : options?.hatch ? 10 : 12;
  const gap = 6;
  let cursor = x;
  ctx.save();
  if (options?.hatch) {
    drawHatchRect(ctx, cursor, Math.round(middleY - 5), swatchW, 10, 1.5, theme);
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
  ctx.fillStyle = rgba(color, 0.16, color);
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
