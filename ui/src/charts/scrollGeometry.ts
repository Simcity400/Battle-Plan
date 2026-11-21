export const MAX_DOM_SCROLL_EXTENT = 2 ** 24;

export interface ScrollGeometry {
  extent: number;
  logicalRange: number;
  physicalRange: number;
  compressed: boolean;
}

const clamp = (value: number, maximum: number) => Math.max(0, Math.min(maximum, value));

export function scrollGeometry(height: number, viewport: number): ScrollGeometry {
  const extent = Math.min(MAX_DOM_SCROLL_EXTENT, Math.max(0, height));
  return { extent, logicalRange: Math.max(0, height - viewport), physicalRange: Math.max(0, extent - viewport), compressed: height > extent };
}

export function physicalScroll(geometry: ScrollGeometry, logical: number): number {
  if (!geometry.compressed) return clamp(logical, geometry.logicalRange);
  return geometry.logicalRange ? clamp(logical, geometry.logicalRange) / geometry.logicalRange * geometry.physicalRange : 0;
}

export function logicalScroll(geometry: ScrollGeometry, physical: number): number {
  if (!geometry.compressed) return clamp(physical, geometry.physicalRange);
  return geometry.physicalRange ? clamp(physical, geometry.physicalRange) / geometry.physicalRange * geometry.logicalRange : 0;
}

export function scrollItemTop(geometry: ScrollGeometry, itemTop: number, logicalTop: number): number {
  return itemTop - logicalTop + physicalScroll(geometry, logicalTop);
}

export function wheelScrollDelta(delta: number, mode: number, lineHeight: number, viewport: number): number {
  return delta * (mode === 1 ? lineHeight : mode === 2 ? viewport : 1);
}
