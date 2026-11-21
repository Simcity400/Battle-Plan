export function placeTooltip(anchor: { x: number; y: number }, size: { width: number; height: number },
  viewport: { width: number; height: number }, offset = 14, margin = 8): { x: number; y: number } {
  let x = anchor.x + offset;
  if (x + size.width > viewport.width - margin) x = anchor.x - offset - size.width;
  let y = anchor.y + offset;
  if (y + size.height > viewport.height - margin) y = anchor.y - offset - size.height;
  x = Math.max(margin, Math.min(x, viewport.width - margin - size.width));
  y = Math.max(margin, Math.min(y, viewport.height - margin - size.height));
  return { x, y };
}
