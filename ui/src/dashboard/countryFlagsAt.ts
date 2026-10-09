import { useCallback } from 'react';
import type { CountryFlagImage, CountryFlagReply, CountryFlagSize } from '../generated/dashboard';
import type { DashboardClient } from './client';
import { FAILED_FACT, MAX_NAME_HOURS } from './countryFacts.ts';
import type { FactSpec, NameAt } from './countryFacts';
import { nameKey, useCountryFactsAt, useCountryFactsRead, useCountryNamesRead } from './countryNamesAt.ts';
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

export type FlagRead = { url?: string; reason?: string };
const readingSpecs = new Map<CountryFlagSize, FactSpec<FlagRead>>();
export function flagReadSpec(size: CountryFlagSize): FactSpec<FlagRead> {
  let spec = readingSpecs.get(size);
  if (!spec) {
    const pixels = flagSpec(size);
    spec = { ...pixels, extract: data => {
      const reply = data as CountryFlagReply;
      const found = new Map<string, FlagRead>();
      for (const [tag, url] of pixels.extract(reply)) found.set(tag, { url });
      for (const missing of reply.unavailable ?? []) if (!found.has(missing.tag)) found.set(missing.tag, { reason: missing.reason });
      return found;
    } };
    readingSpecs.set(size, spec);
  }
  return spec;
}

/** Preserve the supplying read's pending, missing and failure states. */
export function flagReadMetadata(read: FlagRead | null | undefined | typeof FAILED_FACT): { flag?: string; flagPending: boolean; flagReason?: string } {
  return { flag: read && read !== FAILED_FACT ? read.url : undefined, flagPending: read === undefined,
    flagReason: read === FAILED_FACT ? 'The flag request failed.' : read?.reason ?? (read === null ? 'No readable flag was returned for this country at this hour.' : undefined) };
}

/** Country flags as recorded at the given hours (`country-flags`), as image URLs
 * keyed by `nameKey`, held and released as `useCountryFactsAt` holds names. A flag
 * the recording does not give at its hour is absent: nothing shows in its place. */
export function useCountryFlagsAt(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean,
  size: CountryFlagSize = 'small', maxHours = MAX_NAME_HOURS, attempt = 0): ReadonlyMap<string, string> {
  return useCountryFactsAt(client, track, wanted, active, flagSpec(size), maxHours, attempt);
}

/** A country as a tooltip row shows it: its name and flag as recorded at an hour.
 * `known` is absent while pending; false when the name is missing or its read
 * failed. `nameFailed` distinguishes a failed read from a completed missing name. */
export type CountryLabel = (tag: string) => { name: string; flag?: string; known?: boolean; nameFailed?: boolean; flagPending?: boolean; flagReason?: string };

/** Countries named and flagged as the recording showed them at their hours (such as
 * a trade partner at the read behind a tooltip), never a later identity. Only the
 * countries `wanted` are asked for; until their names arrive, or where the
 * recording has none, a country shows by its tag and without a flag. Callers with
 * more distinct drawn hours can pass their own `maxHours`. `revision`
 * changes as names and flags arrive, for a chart to redraw. */
export function useCountryLabelsAt(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean,
  maxHours = MAX_NAME_HOURS) {
  const names = useCountryNamesRead(client, track, wanted, active, maxHours);
  const flags = useCountryFactsRead(client, track, wanted, active, flagReadSpec('small'), maxHours);
  const labelAt = useCallback((hour: number): CountryLabel => tag => {
    const name = names.get(nameKey(hour, tag));
    const flag = flags.get(nameKey(hour, tag));
    return { name: typeof name === 'string' ? name : tag, ...flagReadMetadata(flag),
      known: name === undefined ? undefined : typeof name === 'string', nameFailed: name === FAILED_FACT };
  }, [names, flags]);
  return { labelAt, revision: names.size + flags.size };
}
