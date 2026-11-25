export type Canvas2D = CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D;
export type MeasureText = (text: string, font: string) => number;

type Measuring = Pick<Canvas2D, 'font' | 'measureText'>;

const widths = new Map<string, Map<string, number>>();
const adapters = new WeakMap<Measuring, MeasureText>();
const ABSOLUTE_PX_FONT = /^(?:(?:normal|italic|oblique)\s+)?(?:\d{3}\s+|bold\s+|normal\s+)?\d+(?:\.\d+)?px\s+/;
const MAX_FONTS = 64;
const MAX_TEXTS = 4096;
const LRU_TOUCH_THRESHOLD = 3072;
let watching = false;
let revision = 0;
let cacheMisses = 0;
let lastFont: string | null = null;
let lastBucket: Map<string, number> | null = null;

function fontBucket(font: string): Map<string, number> {
  if (lastFont === font && lastBucket !== null) return lastBucket;
  let byText = widths.get(font);
  if (!byText) {
    if (widths.size >= MAX_FONTS) {
      const oldest = widths.keys().next().value;
      if (oldest !== undefined) {
        widths.delete(oldest);
        if (lastFont === oldest) { lastFont = null; lastBucket = null; }
      }
    }
    byText = new Map();
    widths.set(font, byText);
  }
  lastFont = font;
  lastBucket = byText;
  return byText;
}

function cacheTextWidth(byText: Map<string, number>, text: string, width: number): void {
  if (byText.size >= MAX_TEXTS) {
    const oldest = byText.keys().next().value;
    if (oldest !== undefined) byText.delete(oldest);
  }
  byText.set(text, width);
}

function readCachedWidth(byText: Map<string, number>, text: string): number | undefined {
  const width = byText.get(text);
  if (width !== undefined && byText.size >= LRU_TOUCH_THRESHOLD) {
    byText.delete(text);
    byText.set(text, width);
  }
  return width;
}

export function textWidthRevision(): number {
  return revision;
}

export function textWidthCacheStats(): { readonly fonts: number; readonly misses: number } {
  return { fonts: widths.size, misses: cacheMisses };
}

export function clearTextWidthCache(): void {
  widths.clear();
  lastFont = null;
  lastBucket = null;
  revision += 1;
  cacheMisses = 0;
}

function watchFonts(): void {
  if (watching) return;
  const fonts = typeof document !== 'undefined'
    ? document.fonts
    : typeof self !== 'undefined' && 'fonts' in self
      ? (self as unknown as { fonts?: FontFaceSet }).fonts
      : undefined;
  if (fonts && typeof fonts.addEventListener === 'function') {
    watching = true;
    fonts.addEventListener('loadingdone', () => clearTextWidthCache());
  }
}

export function measureCanvasText(context: Measuring, text: string, font: string): number {
  const fastBucket = lastFont === font ? lastBucket : widths.get(font);
  if (fastBucket !== undefined && fastBucket !== null) {
    const cached = readCachedWidth(fastBucket, text);
    if (cached !== undefined) return cached;
  }
  if (!watching) watchFonts();
  if (ABSOLUTE_PX_FONT.test(font)) {
    const prev = context.font;
    if (prev !== font) context.font = font;
    const width = context.measureText(text).width;
    if (prev !== font) context.font = prev;
    cacheMisses++;
    cacheTextWidth(fastBucket ?? fontBucket(font), text, width);
    return width;
  }
  const prev = context.font;
  if (prev !== font) context.font = font;
  const resolved = context.font;
  const byText = fontBucket(resolved);
  let width = readCachedWidth(byText, text);
  if (width === undefined) {
    width = context.measureText(text).width;
    cacheMisses++;
    cacheTextWidth(byText, text, width);
  }
  if (prev !== font) context.font = prev;
  return width;
}

export function textWidth(context: Measuring, text: string): number {
  const resolved = context.font;
  const byText = fontBucket(resolved);
  let width = readCachedWidth(byText, text);
  if (width === undefined) {
    if (!watching) watchFonts();
    width = context.measureText(text).width;
    cacheMisses++;
    cacheTextWidth(byText, text, width);
  }
  return width;
}

export function canvasMeasureText(context: Measuring): MeasureText {
  let adapter = adapters.get(context);
  if (!adapter) {
    adapter = (text, font) => measureCanvasText(context, text, font);
    adapters.set(context, adapter);
  }
  return adapter;
}
