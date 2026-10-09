/** A prefix that fits its measured line, reserving room for the remainder chip.
 * Only the widths up to the first overflow need measuring. No item-count limit
 * changes what is available in the full tooltip. */
export function fittedListCount(count: number, width: number, itemWidth: (index: number) => number,
  moreWidth: (hidden: number) => number, gap: number): number {
  if (count <= 0 || width <= 0) return 0;
  const prefixes = [0];
  for (let index = 0; index < count; index++) {
    const next = prefixes[index] + (index ? gap : 0) + itemWidth(index);
    if (next > width) break;
    prefixes.push(next);
  }
  if (prefixes.length - 1 === count) return count;
  for (let shown = prefixes.length - 1; shown >= 0; shown--) {
    if (prefixes[shown] + (shown ? gap : 0) + moreWidth(count - shown) <= width) return shown;
  }
  return 0;
}

/** The full remainder stays available on pointer hover and keyboard focus. */
export function fittedListRemainder<T>(items: readonly T[], shown: number, label: (item: T) => string): string {
  return items.slice(shown).map(label).join('\n');
}
