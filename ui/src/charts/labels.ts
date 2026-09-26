/** Label placement shared by every chart. */

export interface EndLabel { id: string; y: number; height: number }

/** End labels at the right of a chart: each stays as close to its line's end as it
 * can without overlapping another, keeping their order and inside [top, bottom].
 * When they cannot all fit, they start at the top and the last ones run past the
 * bottom rather than above the plot. */
export function layoutEndLabels(labels: readonly EndLabel[], top: number, bottom: number, gap = 2): Map<string, number> {
  const sorted = [...labels].sort((a, b) => a.y - b.y || a.id.localeCompare(b.id));
  const placed = sorted.map(label => ({ ...label, at: label.y }));
  // Push down from the top, then back up from the bottom.
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

/** A label column only as wide as its longest text over the whole campaign, so it
 * never jumps during playback; clamped to [min, max]. */
export function fitLabelWidth(texts: Iterable<string>, measure: (text: string) => number, min: number, max: number, padding = 0): number {
  let widest = 0;
  for (const text of texts) widest = Math.max(widest, measure(text));
  return Math.ceil(Math.min(max, Math.max(min, widest + padding)));
}
