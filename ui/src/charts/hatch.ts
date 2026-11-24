import type { Canvas2D } from './textWidth.ts';
import type { ChartTokens } from './tokens';

export function hatchTile(spacing: number) {
  const size = Math.max(2, spacing);
  return { size, lines: [[0, size, size, 0]] };
}

export function fillHatched(ctx: Canvas2D, x: number, y: number, width: number, height: number,
  color: string, spacing = 4, lineWidth = 1.5): void {
  if (width <= 0 || height <= 0) return;
  ctx.save(); ctx.beginPath(); ctx.rect(x, y, width, height); ctx.clip();
  const alpha = ctx.globalAlpha;
  ctx.globalAlpha = alpha * 0.25; ctx.fillStyle = color; ctx.fillRect(x, y, width, height);
  ctx.globalAlpha = alpha; ctx.strokeStyle = color; ctx.lineWidth = lineWidth;
  ctx.beginPath();
  const size = hatchTile(spacing).size;
  for (let start = x - height; start < x + width; start += size) {
    ctx.moveTo(start, y + height); ctx.lineTo(start + height, y);
  }
  ctx.stroke(); ctx.restore();
}

export function drawUnknownHatch(ctx: Canvas2D, tokens: ChartTokens,
  rect: { left: number; top: number; width: number; height: number }): void {
  fillHatched(ctx, rect.left, rect.top, rect.width, rect.height, tokens.muted, tokens.hatchSpacing, tokens.lineWidth.grid);
}
