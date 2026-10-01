import { useCallback } from 'react';
import type { CountryFlagImage, CountryFlagReply, CountryFlagSize } from '../generated/dashboard';
import type { DashboardClient } from './client';
import { MAX_NAME_HOURS } from './countryFacts.ts';
import type { FactSpec, NameAt } from './countryFacts';
import { nameKey, useCountryFactsAt, useCountryNamesAt } from './countryNamesAt.ts';
import type { BoundTrack } from './model';

/** A flag's pixels as an image URL, for places that take an image (tooltip rows). */
export function flagImageUrl(image: CountryFlagImage): string | undefined {
  if (typeof document === 'undefined') return undefined;
  const canvas = document.createElement('canvas');
  canvas.width = image.width;
  canvas.height = image.height;
  const context = canvas.getContext('2d');
  if (!context) return undefined;
  context.putImageData(new ImageData(Uint8ClampedArray.from(image.pixels), image.width, image.height), 0, 0);
  return canvas.toDataURL();
}

/** One reading spec per flag size (a `country-flags` request draws at most 32 countries). */
const specs = new Map<CountryFlagSize, FactSpec<string>>();
export function flagSpec(size: CountryFlagSize): FactSpec<string> {
  let spec = specs.get(size);
  if (!spec) {
    spec = {
      product: 'country-flags', perRequest: 32,
      params: (branchId, hour, tags) => ({ branchId, hour, tags, size }),
      extract: data => new Map((data as CountryFlagReply).flags.flatMap(image => {
        const url = flagImageUrl(image);
        return url ? [[image.tag, url] as const] : [];
      })),
    };
    specs.set(size, spec);
  }
  return spec;
}

/** Country flags as recorded at the given hours (`country-flags`), as image URLs
 * keyed by `nameKey`, held and released as `useCountryFactsAt` holds names. A flag
 * the recording does not give at its hour is absent: nothing shows in its place. */
export function useCountryFlagsAt(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean,
  size: CountryFlagSize = 'small', maxHours = MAX_NAME_HOURS, attempt = 0): ReadonlyMap<string, string> {
  return useCountryFactsAt(client, track, wanted, active, flagSpec(size), maxHours, attempt);
}

/** A country as a tooltip row shows it: its name and flag as recorded at an hour. */
export type CountryLabel = (tag: string) => { name: string; flag?: string };

/** Countries named and flagged as the recording showed them at their hours (such as
 * a trade partner at the read behind a tooltip), never a later identity. Only the
 * countries `wanted` are asked for; until their names arrive, or where the
 * recording has none, a country shows by its tag and without a flag. `revision`
 * changes as names and flags arrive, for a chart to redraw. */
export function useCountryLabelsAt(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean) {
  const names = useCountryNamesAt(client, track, wanted, active);
  const flags = useCountryFlagsAt(client, track, wanted, active);
  const labelAt = useCallback((hour: number): CountryLabel => tag => ({ name: names.get(nameKey(hour, tag)) ?? tag, flag: flags.get(nameKey(hour, tag)) }), [names, flags]);
  return { labelAt, revision: names.size + flags.size };
}
