export const MAX_PHYSICAL_EXTENT_PX = 8_000_000;

export interface ScrollGeometry {
  readonly logicalHeight: number;
  readonly physicalHeight: number;
  readonly viewportHeight: number;
  readonly compressed: boolean;
}

export function computeScrollGeometry(
  logicalHeight: number,
  viewportHeight: number,
  maxPhysicalPx = MAX_PHYSICAL_EXTENT_PX,
): ScrollGeometry {
  const vh = Math.max(0, viewportHeight);
  const logical = Math.max(vh, logicalHeight);
  const cap = Math.max(vh, maxPhysicalPx);
  if (logical <= cap) {
    return { logicalHeight: logical, physicalHeight: logical, viewportHeight: vh, compressed: false };
  }
  return { logicalHeight: logical, physicalHeight: cap, viewportHeight: vh, compressed: true };
}

export function physicalToLogicalTop(physicalTop: number, geometry: ScrollGeometry): number {
  const maxLogical = Math.max(0, geometry.logicalHeight - geometry.viewportHeight);
  if (!geometry.compressed) return Math.max(0, Math.min(maxLogical, physicalTop));
  const maxPhysical = Math.max(1, geometry.physicalHeight - geometry.viewportHeight);
  const ratio = Math.max(0, Math.min(1, physicalTop / maxPhysical));
  return ratio * maxLogical;
}

export function logicalToPhysicalTop(logicalTop: number, geometry: ScrollGeometry): number {
  const maxLogical = Math.max(0, geometry.logicalHeight - geometry.viewportHeight);
  if (!geometry.compressed || maxLogical <= 0) return Math.max(0, Math.min(maxLogical, logicalTop));
  const maxPhysical = Math.max(0, geometry.physicalHeight - geometry.viewportHeight);
  const ratio = Math.max(0, Math.min(1, logicalTop / maxLogical));
  return ratio * maxPhysical;
}

export function adjustScrollForPrependedRows(
  physicalScrollTop: number,
  prependedRows: number,
  rowHeight: number,
  previousGeometry: ScrollGeometry,
  nextGeometry: ScrollGeometry,
): number {
  if (physicalScrollTop <= 0 || prependedRows <= 0 || rowHeight <= 0) return physicalScrollTop;
  const previousLogical = physicalToLogicalTop(physicalScrollTop, previousGeometry);
  return logicalToPhysicalTop(previousLogical + prependedRows * rowHeight, nextGeometry);
}

export function scrollRangeIntoView(
  node: HTMLElement | null,
  viewport: { scrollTop: number; height: number },
  logicalTop: number,
  itemHeight: number,
  geometry?: ScrollGeometry,
): boolean {
  if (!node || logicalTop < 0 || viewport.height <= 0) return false;
  const currentLogical = geometry ? physicalToLogicalTop(viewport.scrollTop, geometry) : viewport.scrollTop;
  const bottom = logicalTop + itemHeight;
  let nextLogical = currentLogical;
  if (logicalTop < currentLogical) nextLogical = logicalTop;
  else if (bottom > currentLogical + viewport.height) nextLogical = bottom - viewport.height;
  if (nextLogical === currentLogical) return false;
  const nextPhysical = geometry ? logicalToPhysicalTop(nextLogical, geometry) : nextLogical;
  if (nextPhysical === viewport.scrollTop) return false;
  node.scrollTop = nextPhysical;
  viewport.scrollTop = nextPhysical;
  return true;
}
