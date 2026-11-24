import { compareTags } from '../../dashboard/tagOrder.ts';
import type { SunburstItem, SunburstKey } from './layout';
import type { TooltipContent } from '../tooltip';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';

export interface ShareValue { key: string; value: number | null }
export interface ShareReading {
  items: SunburstItem[];
  unavailableReason: string | null;
  missingLabel: 'Not recorded' | 'Share unavailable';
}

export function shareColourReason(keys: readonly string[], colourOf: (key: string) => string | null | undefined): string | null {
  return keys.some(key => colourOf(key) == null) ? "The selected slices' colours were not recorded at this hour." : null;
}

export function shareTooltip(forms: readonly TooltipContent[], label: ReturnType<CountryLabel> | undefined,
  subtitle: string, note?: string): TooltipContent[] {
  return forms.map(form => ({ ...form, title: label?.name ?? form.title, icon: label ? undefined : form.icon,
    countryFlag: label ?? form.countryFlag, subtitle, note: note ?? form.note }));
}

export function shareReading(values: readonly ShareValue[], world: number | null, count: number,
  look: (key: string) => SunburstKey, rest: SunburstKey, reason?: string | null): ShareReading {
  const missing = (unavailableReason: string, unsupported = false): ShareReading =>
    ({ items: [], unavailableReason, missingLabel: unsupported ? 'Share unavailable' : 'Not recorded' });
  if (reason != null) return missing(reason);
  if (world == null || values.some(part => part.value == null)) return missing('The whole share was not recorded at this hour.');
  if (!Number.isSafeInteger(world) || values.some(part => !Number.isSafeInteger(part.value))) return missing('The share exceeds the exact integer range.');
  if (world < 0 || values.some(part => part.value! < 0)) return missing('A share cannot represent negative recorded amounts. Choose Race or Lines over time to see them.', true);
  const leaders = values.slice().sort((a, b) => b.value! - a.value! || compareTags(a.key, b.key)).slice(0, Math.max(0, Math.floor(count)));
  let total = 0;
  for (const part of leaders) {
    total += part.value!;
    if (!Number.isSafeInteger(total)) return missing('The share exceeds the exact integer range.');
  }
  if (total > world) return missing('These recorded totals exceed the world total and cannot form a share.', true);
  const items: SunburstItem[] = leaders.map(part => ({ path: [{ ...look(part.key), keepZero: true }], value: part.value!, exact: BigInt(part.value!) }));
  items.push({ path: [{ ...rest, last: true, keepZero: true }], value: world - total, exact: BigInt(world - total) });
  return { items, unavailableReason: null, missingLabel: 'Not recorded' };
}
