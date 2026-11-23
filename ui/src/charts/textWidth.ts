export type Canvas2D = CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D;
export type MeasureText = (text: string, font: string) => number;

type Measuring = Pick<Canvas2D, 'font' | 'measureText'>;

const widths = new Map<string, Map<string, number>>();
const adapters = new WeakMap<Measuring, MeasureText>();
const ABSOLUTE_PX_FONT = /^(?:(?:normal|italic|oblique)\s+)?(?:\d{3}\s+|bold\s+|normal\s+)?\d+(?:\.\d+)?px\s+/;
const MAX_TEXTS = 4096;
let watching = false;
let revision = 0;

export function textWidthRevision(): number {
  return revision;
}

export function clearTextWidthCache(): void {
  widths.clear();
  revision += 1;
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
  watchFonts();
  if (ABSOLUTE_PX_FONT.test(font)) {
    let byText = widths.get(font);
    let width = byText?.get(text);
    if (width === undefined) {
      const prev = context.font;
      if (prev !== font) context.font = font;
      width = context.measureText(text).width;
      if (prev !== font) context.font = prev;
      if (!byText) widths.set(font, byText = new Map());
      if (byText.size >= MAX_TEXTS) byText.clear();
      byText.set(text, width);
    }
    return width;
  }
  const prev = context.font;
  if (prev !== font) context.font = font;
  const resolved = context.font;
  let byText = widths.get(resolved);
  let width = byText?.get(text);
  if (width === undefined) {
    width = context.measureText(text).width;
    if (!byText) widths.set(resolved, byText = new Map());
    if (byText.size >= MAX_TEXTS) byText.clear();
    byText.set(text, width);
  }
  if (prev !== font) context.font = prev;
  return width;
}

export function textWidth(context: Measuring, text: string): number {
  watchFonts();
  const resolved = context.font;
  let byText = widths.get(resolved);
  let width = byText?.get(text);
  if (width === undefined) {
    width = context.measureText(text).width;
    if (!byText) widths.set(resolved, byText = new Map());
    if (byText.size >= MAX_TEXTS) byText.clear();
    byText.set(text, width);
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
