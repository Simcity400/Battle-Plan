import type { ChartTokens } from '../tokens';

/** Disclosure marks share the label's text colour in either orientation. */
export function drawDisclosure(context: Pick<CanvasRenderingContext2D, 'fillStyle' | 'beginPath' | 'moveTo' | 'lineTo' | 'closePath' | 'fill'>,
  x: number, y: number, expanded: boolean, tokens: Pick<ChartTokens, 'text'>): void {
  context.fillStyle = tokens.text; context.beginPath();
  if (expanded) { context.moveTo(x - 4, y - 2); context.lineTo(x + 4, y - 2); context.lineTo(x, y + 3); }
  else { context.moveTo(x - 2, y - 4); context.lineTo(x + 3, y); context.lineTo(x - 2, y + 4); }
  context.closePath(); context.fill();
}

/** Event crosses do not depend on the chosen font containing a glyph. */
export function drawCross(context: Pick<CanvasRenderingContext2D, 'strokeStyle' | 'lineWidth' | 'beginPath' | 'moveTo' | 'lineTo' | 'stroke'>,
  x: number, y: number, color: string): void {
  context.strokeStyle = color;
  context.lineWidth = 2;
  context.beginPath();
  context.moveTo(x - 4, y - 4); context.lineTo(x + 4, y + 4);
  context.moveTo(x + 4, y - 4); context.lineTo(x - 4, y + 4);
  context.stroke();
}

/** The second click of a double-click belongs to Reset zoom, not expansion. */
export const acceptsLaneClick = (detail: number, dragged: boolean) => !dragged && detail <= 1;

/** Plot menus are the default; a caller can explicitly offer label actions. */
export const acceptsLaneMenu = (x: number, left: number, labelMenu = false) => x >= left || labelMenu;
