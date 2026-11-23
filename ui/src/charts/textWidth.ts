export type Canvas2D = CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D;
export type MeasureText = (text: string, font: string) => number;

type Measuring = Pick<Canvas2D, 'font' | 'measureText'>;

const widths = new Map<string, Map<string, number>>();
const MAX_TEXTS = 4096;
let watching = false;

export function clearTextWidthCache(): void {
  widths.clear();
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
    fonts.addEventListener('loadingdone', () => widths.clear());
  }
}

export function measureCanvasText(context: Measuring, text: string, font: string): number {
  watchFonts();
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

export function textWidth(context: Measuring, text: string): number {
  return measureCanvasText(context, text, context.font);
}

export function canvasMeasureText(context: Measuring): MeasureText {
  return (text, font) => measureCanvasText(context, text, font);
}
