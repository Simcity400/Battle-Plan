import type { DisplayedHour, EntityKind, FieldStatus, IdentityProvider, IdentityResult } from './plan.ts';

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

export function countryLooksIdentityProvider(looks: CountryLooksMap | undefined): IdentityProvider {
  return {
    resolve(kind: EntityKind, key: string, atHour: DisplayedHour): IdentityResult {
      if (kind !== 'country' || !looks) {
        return {
          kind,
          key,
          atHour,
          holder: { status: 'pending' },
          name: { status: 'pending' },
          validInterval: [atHour, atHour],
          nextChangeAt: Infinity,
        };
      }
      const runs = looks[key];
      if (!runs || !runs.length) {
        return {
          kind,
          key,
          atHour,
          holder: { status: 'ready', value: key },
          tag: { status: 'ready', value: key },
          name: { status: 'pending' },
          colour: { status: 'pending' },
          flagChoice: { status: 'pending' },
          validInterval: [atHour, atHour],
          nextChangeAt: Infinity,
        };
      }
      let low = 0;
      let high = runs.length;
      while (low < high) {
        const mid = (low + high) >> 1;
        if (runs[mid]!.from <= atHour) low = mid + 1;
        else high = mid;
      }
      const look = runs[low - 1];
      const next = runs[low];
      const nextChangeAt = next ? next.from : Infinity;
      const validLo = look ? look.from : -Infinity;
      const validHi = Number.isFinite(nextChangeAt) ? nextChangeAt - 1 : Infinity;
      const name: FieldStatus<string> = look?.name && look.name.trim()
        ? { status: 'ready', value: look.name }
        : look?.status
          ? { status: 'absent', reason: look.status }
          : { status: 'pending' };
      const colour: FieldStatus<string> = look?.color
        ? { status: 'ready', value: look.color }
        : look
          ? { status: 'absent', reason: 'No map colour recorded' }
          : { status: 'pending' };
      const flagChoice: FieldStatus<string> = look?.flag
        ? { status: 'ready', value: look.flag }
        : look
          ? { status: 'absent', reason: look.status ?? 'Flag unavailable' }
          : { status: 'pending' };
      return {
        kind,
        key,
        atHour,
        holder: { status: 'ready', value: key },
        tag: { status: 'ready', value: key },
        name,
        colour,
        flagChoice,
        validInterval: [validLo, Math.max(validLo, validHi)],
        nextChangeAt,
      };
    },
  };
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
