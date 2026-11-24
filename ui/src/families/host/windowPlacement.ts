export const DETAIL_WINDOW_MARGIN = 12;

export function clampWindowPosition(
  left: number,
  top: number,
  width: number,
  height: number,
  viewportWidth: number,
  viewportHeight: number,
  margin = DETAIL_WINDOW_MARGIN,
): { readonly left: number; readonly top: number } {
  const maxLeft = Math.max(margin, viewportWidth - width - margin);
  const maxTop = Math.max(margin, viewportHeight - height - margin);
  return {
    left: Math.round(Math.max(margin, Math.min(left, maxLeft))),
    top: Math.round(Math.max(margin, Math.min(top, maxTop))),
  };
}

export function centerWindowPosition(
  width: number,
  height: number,
  viewportWidth: number,
  viewportHeight: number,
  margin = DETAIL_WINDOW_MARGIN,
): { readonly left: number; readonly top: number } {
  return clampWindowPosition(
    (viewportWidth - width) / 2,
    (viewportHeight - height) / 2,
    width,
    height,
    viewportWidth,
    viewportHeight,
    margin,
  );
}
