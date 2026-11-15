/** Keyboard navigation shared by categorical charts and logs. */
export function keyTarget(key: string, index: number, count: number, page: number): number | null {
  if (count <= 0) return null;
  const clamp = (value: number) => Math.max(0, Math.min(count - 1, value));
  switch (key) {
    case 'ArrowDown': return clamp(index + 1);
    case 'ArrowUp': return clamp(index - 1);
    case 'PageDown': return clamp(index + Math.max(1, page));
    case 'PageUp': return clamp(index - Math.max(1, page));
    case 'Home': return 0;
    case 'End': return count - 1;
    default: return null;
  }
}

/** The first slot from `from` (clamped into `[0, count)`), stepping by `step`, that `shown` admits; null when none is. */
export function shownSlot(from: number, step: 1 | -1, count: number, shown: (slot: number) => boolean): number | null {
  for (let slot = Math.max(0, Math.min(count - 1, from)); slot >= 0 && slot < count; slot += step) if (shown(slot)) return slot;
  return null;
}
