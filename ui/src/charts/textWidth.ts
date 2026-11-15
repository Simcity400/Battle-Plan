/** Canvas text widths, measured once per font and text, so a frame of playback measures only text it has never drawn in
 * that font. A font that finishes loading changes what its family measures, so every width is let go then. */
type Measuring = Pick<CanvasRenderingContext2D, 'font' | 'measureText'>;

const widths = new Map<string, Map<string, number>>();
/** The most texts kept for one font; past it the font's widths start again. */
const MAX_TEXTS = 4096;
let watching = false;

/** The width of `text` in `context`'s font now, in layout pixels. */
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
