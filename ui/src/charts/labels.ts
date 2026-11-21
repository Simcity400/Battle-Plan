
export interface EndLabel { id: string; y: number; height: number }

export function layoutEndLabels(labels: readonly EndLabel[], top: number, bottom: number, gap = 2): Map<string, number> {
  const sorted = [...labels].sort((a, b) => a.y - b.y || a.id.localeCompare(b.id));
  const placed = sorted.map(label => ({ ...label, at: label.y }));
  let floor = top;
  for (const label of placed) {
    label.at = Math.max(label.at, floor + label.height / 2);
    floor = label.at + label.height / 2 + gap;
  }
  let ceiling = bottom;
  for (let index = placed.length - 1; index >= 0; index--) {
    const label = placed[index];
    label.at = Math.min(label.at, ceiling - label.height / 2);
    ceiling = label.at - label.height / 2 - gap;
  }
  floor = top;
  for (const label of placed) {
    label.at = Math.max(label.at, floor + label.height / 2);
    floor = label.at + label.height / 2 + gap;
  }
  return new Map(placed.map(label => [label.id, label.at]));
}

export function fitLabelWidth<T = string>(texts: Iterable<T>, measure: (text: T) => number, min: number, max: number, padding = 0): number {
  let widest = 0;
  for (const text of texts) {
    widest = Math.max(widest, measure(text));
    if (widest + padding >= max) break;
  }
  return Math.ceil(Math.min(max, Math.max(min, widest + padding)));
}

export function ellipsise(text: string, width: number, measure: (text: string) => number): string {
  if (measure(text) <= width) return text;
  if (measure('\u2026') > width) return '';
  let lo = 0, hi = text.length;
  while (lo < hi) { const mid = Math.ceil((lo + hi) / 2); if (measure(text.slice(0, mid) + '\u2026') <= width) lo = mid; else hi = mid - 1; }
  return text.slice(0, lo) + '\u2026';
}
