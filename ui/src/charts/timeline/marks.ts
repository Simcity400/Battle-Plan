import type { ChartTokens } from '../tokens';

export function drawDisclosure(context: Pick<CanvasRenderingContext2D, 'fillStyle' | 'beginPath' | 'moveTo' | 'lineTo' | 'closePath' | 'fill'>,
  x: number, y: number, expanded: boolean, tokens: Pick<ChartTokens, 'text'>): void {
  context.fillStyle = tokens.text; context.beginPath();
  if (expanded) { context.moveTo(x - 4, y - 2); context.lineTo(x + 4, y - 2); context.lineTo(x, y + 3); }
  else { context.moveTo(x - 2, y - 4); context.lineTo(x + 3, y); context.lineTo(x - 2, y + 4); }
  context.closePath(); context.fill();
}

export function drawCross(context: Pick<CanvasRenderingContext2D, 'strokeStyle' | 'lineWidth' | 'beginPath' | 'moveTo' | 'lineTo' | 'stroke'>,
  x: number, y: number, color: string): void {
  context.strokeStyle = color;
  context.lineWidth = 2;
  context.beginPath();
  context.moveTo(x - 4, y - 4); context.lineTo(x + 4, y + 4);
  context.moveTo(x + 4, y - 4); context.lineTo(x - 4, y + 4);
  context.stroke();
}

export const acceptsLaneClick = (detail: number, dragged: boolean) => !dragged && detail <= 1;

export const acceptsLaneMenu = (x: number, left: number, labelMenu = false) => x >= left || labelMenu;
