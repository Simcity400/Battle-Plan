type Measuring = Pick<CanvasRenderingContext2D, 'font' | 'measureText'>;

const widths = new Map<string, Map<string, number>>();
const MAX_TEXTS = 4096;
let watching = false;

export function textWidth(context: Measuring, text: string): number {
  if (!watching && typeof document !== 'undefined' && document.fonts) {
    watching = true;
    document.fonts.addEventListener('loadingdone', () => widths.clear());
  }
  const font = context.font;
  let byText = widths.get(font);
  if (!byText) widths.set(font, byText = new Map());
  let width = byText.get(text);
  if (width === undefined) {
    if (byText.size >= MAX_TEXTS) byText.clear();
    byText.set(text, width = context.measureText(text).width);
  }
  return width;
}
