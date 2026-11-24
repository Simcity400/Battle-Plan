export function readAt(reads: readonly number[], hour: number): number | null {
  let low = 0;
  let high = reads.length - 1;
  let found: number | null = null;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (reads[middle] <= hour) { found = reads[middle]; low = middle + 1; } else high = middle - 1;
  }
  return found;
}

export function entryAt<T extends { firstHour: number }>(entries: readonly T[], hour: number): T | null {
  let low = 0;
  let high = entries.length - 1;
  let found: T | null = null;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (entries[middle].firstHour <= hour) { found = entries[middle]; low = middle + 1; } else high = middle - 1;
  }
  return found;
}

