export interface CountryLook {
  from: number;
  name: string | null;
  flag?: string;
  color?: string;
  status?: string;
}

export type CountryLooksMap = Readonly<Record<string, readonly CountryLook[]>>;

export function lookAt(looks: readonly CountryLook[] | undefined, hour: number): CountryLook | undefined {
  if (!looks) return undefined;
  let low = 0, high = looks.length;
  while (low < high) {
    const middle = (low + high) >> 1;
    if (looks[middle]!.from <= hour) low = middle + 1;
    else high = middle;
  }
  return looks[low - 1];
}

const lookBoundsCache = new WeakMap<object, readonly number[]>();

export function nextLookChange(looks: CountryLooksMap | undefined, hour: number): number {
  if (!looks) return Infinity;
  let bounds = lookBoundsCache.get(looks);
  if (!bounds) {
    const set = new Set<number>();
    for (const key in looks) {
      const runs = looks[key];
      if (!runs) continue;
      for (const run of runs) set.add(run.from);
    }
    bounds = [...set].sort((a, b) => a - b);
    lookBoundsCache.set(looks, bounds);
  }
  let low = 0, high = bounds.length;
  while (low < high) {
    const mid = (low + high) >> 1;
    if (bounds[mid]! <= hour) low = mid + 1;
    else high = mid;
  }
  return bounds[low] ?? Infinity;
}
