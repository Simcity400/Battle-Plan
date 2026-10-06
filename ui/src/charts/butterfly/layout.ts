/** Geometry for paired bars around a centre label column. */
export interface ButterflyValue { id: string; left: number | null; right: number | null }
export interface ButterflyLayout {
  left: number; right: number; half: number; labelWidth: number; ratioX: number;
  top: number; slot: number; barHeight: number;
}

/** Both halves keep the same room, with exact end labels and a separate ratio column. */
export function butterflyLayout(width: number, height: number, count: number, fontSize: number,
  labelWidth: number, valueWidth: number, ratioWidth: number): ButterflyLayout {
  const gap = fontSize / 2;
  const edge = fontSize * 2 / 3;
  const ratioRoom = ratioWidth ? ratioWidth + gap * 2 : 0;
  const left = width / 2 - labelWidth / 2;
  const half = Math.max(0, left - edge - valueWidth - gap - ratioRoom);
  const right = left + labelWidth;
  const top = fontSize * 2.5;
  const slot = Math.min(fontSize * 3.5, Math.max(fontSize * 1.8, (height - top - fontSize) / Math.max(1, count)));
  return { left, right, half, labelWidth, ratioX: width - edge, top, slot, barHeight: Math.min(20, slot * 0.7) };
}

/** One fitted symmetric scale. Missing and unsupported negative values do not fit it. */
export function butterflyScale(items: readonly ButterflyValue[]): number {
  let maximum = 0;
  for (const item of items) for (const value of [item.left, item.right]) if (value != null && Number.isFinite(value)) maximum = Math.max(maximum, value);
  return maximum > 0 ? maximum * 1.05 : 1;
}

/** Caller text plus optional flag room; reserve at most 32% for labels. */
export function butterflyLabelWidth(width: number, fontSize: number, longestName: number, showFlags = true): number {
  return Math.min(width * 0.32, longestName + fontSize * (showFlags ? 3 : 1));
}

/** Truncate glyphs to fit, never squeeze them with Canvas fillText's maxWidth. */
export function ellipsizeLabel(label: string, width: number, measure: (text: string) => number): string {
  if (measure(label) <= width) return label;
  if (measure('…') > width) return '';
  const glyphs = Array.from(label);
  let lo = 0, hi = glyphs.length;
  while (lo < hi) {
    const mid = Math.ceil((lo + hi) / 2);
    if (measure(glyphs.slice(0, mid).join('') + '…') <= width) lo = mid; else hi = mid - 1;
  }
  return glyphs.slice(0, lo).join('') + '…';
}

/** The right/left ratio, rounded to two decimals; a zero or missing denominator has none. */
export function formatRatio(right: number | bigint | null, left: number | bigint | null): string {
  if (right == null || left == null || left === 0 || left === 0n) return '—';
  if (typeof right === 'bigint' && typeof left === 'bigint') {
    const numerator = right < 0n ? -right : right, denominator = left < 0n ? -left : left;
    const hundredths = (numerator * 200n + denominator) / (denominator * 2n);
    const sign = (right < 0n) !== (left < 0n) && hundredths !== 0n ? '-' : '';
    return `${sign}${hundredths / 100n}.${String(hundredths % 100n).padStart(2, '0')} : 1`;
  }
  const ratio = Number(right) / Number(left);
  return Number.isFinite(ratio) ? `${ratio.toFixed(2)} : 1` : '—';
}

/** The list fills the body until its rows need scrolling at the minimum slot. */
export function butterflyMinimumHeight(count: number, fontSize: number): number {
  return fontSize * (2.5 + count * 1.8);
}

/** The complete list scrolls, while the backing canvas stays viewport-sized. */
export function butterflySurface(count: number, fontSize: number, viewportHeight: number) {
  return { canvasHeight: viewportHeight, contentHeight: Math.max(viewportHeight, butterflyMinimumHeight(count, fontSize)) };
}

/** Visible list coordinates below the fixed column headers. */
export function butterflyRowViewport(top: number, viewportHeight: number, scrollTop: number) {
  return { start: scrollTop + top, end: scrollTop + viewportHeight, height: Math.max(0, viewportHeight - top) };
}

/** Measure caller text without spreading an unbounded item list into arguments. */
export function butterflyTextWidths(items: Iterable<{ leftText: string; rightText: string; ratioText: string }>,
  measure: (text: string) => number, ratioLabel: string, showValues: boolean, showRatio: boolean) {
  let valueWidth = 0, ratioWidth = showRatio ? measure(ratioLabel) : 0;
  for (const item of items) {
    if (showValues) valueWidth = Math.max(valueWidth, measure(item.leftText), measure(item.rightText));
    if (showRatio) ratioWidth = Math.max(ratioWidth, measure(item.ratioText));
  }
  return { valueWidth, ratioWidth };
}
