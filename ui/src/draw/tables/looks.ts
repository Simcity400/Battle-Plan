export interface CountryLook { from: number; name: string | null; flag?: string; status?: string }

export function lookAt(looks: readonly CountryLook[] | undefined, hour: number): CountryLook | undefined {
  if (!looks) return undefined;
  let low = 0, high = looks.length;
  while (low < high) { const middle = (low + high) >> 1; if (looks[middle]!.from <= hour) low = middle + 1; else high = middle; }
  return looks[low - 1];
}
