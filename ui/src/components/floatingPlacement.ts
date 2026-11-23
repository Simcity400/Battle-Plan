export const MARGIN = 12;

export const BAR = 48;

export function clampPanel(width: number, height: number, x: number, y: number, view: { width: number; height: number }, shown: number) {
  return { x: Math.round(Math.min(Math.max(MARGIN, x), Math.max(MARGIN, view.width - width - MARGIN))),
    y: Math.round(Math.min(Math.max(MARGIN, y), Math.max(MARGIN, view.height - Math.min(height, shown) - MARGIN))) };
}
