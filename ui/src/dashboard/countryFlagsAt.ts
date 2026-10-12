import { useCallback } from 'react';
import type { CountryFlagImage, CountryFlagReply, CountryFlagSize } from '../generated/dashboard';
import type { DashboardClient } from './client';
import { FAILED_FACT, MAX_NAME_HOURS } from './countryFacts.ts';
import type { FactSpec, NameAt } from './countryFacts';
import { nameKey, useCountryFactsAt, useCountryFactsRead, useCountryNamesRead } from './countryNamesAt.ts';
import type { BoundTrack } from './model';

/** Encoded flags, shared by every widget and hour showing the same artwork, least
 * recently used dropped past this many UTF-16 bytes. Holders keep their own string,
 * so dropping one never blanks a flag shown. */
const ENCODED_BYTES = 16 * 1024 * 1024;
const encoded = new Map<string, { url: Promise<string | undefined>; bytes: number }>();
let encodedBytes = 0;

function encode(image: CountryFlagImage): Promise<string | undefined> {
  if (typeof document === 'undefined') return Promise.resolve(undefined);
  const canvas = document.createElement('canvas');
  canvas.width = image.width;
  canvas.height = image.height;
  const context = canvas.getContext('2d');
  if (!context) return Promise.resolve(undefined);
  context.putImageData(new ImageData(Uint8ClampedArray.from(image.pixels), image.width, image.height), 0, 0);
  // The PNG is compressed off the main thread, then read back as a data URL.
  return new Promise(resolve => canvas.toBlob(blob => {
    if (!blob) { resolve(undefined); return; }
    const reader = new FileReader();
    reader.onload = () => resolve(typeof reader.result === 'string' ? reader.result : undefined);
    reader.onerror = () => resolve(undefined);
    reader.readAsDataURL(blob);
  }));
}

/** A flag's pixels as an image URL, for places that take an image (tooltip rows):
 * encoded once per `scope` (recording, content and size) and artwork. */
export function flagImageUrl(image: CountryFlagImage, scope = ''): Promise<string | undefined> {
  const key = JSON.stringify([scope, image.asset, image.width, image.height]);
  const known = encoded.get(key);
  if (known) { encoded.delete(key); encoded.set(key, known); return known.url; }
  const entry = { url: encode(image), bytes: 0 };
  encoded.set(key, entry);
  void entry.url.then(url => {
    if (encoded.get(key) !== entry) return;
    entry.bytes = (url?.length ?? 0) * 2;
    encodedBytes += entry.bytes;
    for (const [oldest, held] of encoded) {
      if (encodedBytes <= ENCODED_BYTES || oldest === key) break;
      encoded.delete(oldest);
      encodedBytes -= held.bytes;
    }
  });
  return entry.url;
}

/** One reading spec per flag size (a `country-flags` request draws at most 32 countries). */
const specs = new Map<CountryFlagSize, FactSpec<string, Promise<ReadonlyMap<string, string>>>>();
export function flagSpec(size: CountryFlagSize): FactSpec<string, Promise<ReadonlyMap<string, string>>> {
  let spec = specs.get(size);
  if (!spec) {
    spec = {
      product: 'country-flags', perRequest: 32,
      params: (branchId, hour, tags) => ({ branchId, hour, tags, size }),
      extract: async (data, _params, source) => {
        const scope = JSON.stringify([source?.sourceId, source?.revision, source?.content, size]);
        const flags = (data as CountryFlagReply).flags;
        const urls = await Promise.all(flags.map(image => flagImageUrl(image, scope)));
        return new Map(flags.flatMap((image, index) => {
          const url = urls[index];
          return url ? [[image.tag, url] as const] : [];
        }));
      },
    };
    specs.set(size, spec);
  }
  return spec;
}

export type FlagRead = { url?: string; reason?: string };
const readingSpecs = new Map<CountryFlagSize, FactSpec<FlagRead, Promise<ReadonlyMap<string, FlagRead>>>>();
export function flagReadSpec(size: CountryFlagSize): FactSpec<FlagRead, Promise<ReadonlyMap<string, FlagRead>>> {
  let spec = readingSpecs.get(size);
  if (!spec) {
    const pixels = flagSpec(size);
    spec = { ...pixels, extract: async (data, params, source) => {
      const reply = data as CountryFlagReply;
      const found = new Map<string, FlagRead>();
      for (const [tag, url] of await pixels.extract(reply, params, source)) found.set(tag, { url });
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
  size: CountryFlagSize = 'panel', maxHours = MAX_NAME_HOURS, attempt = 0): ReadonlyMap<string, string> {
  return useCountryFactsAt(client, track, wanted, active, flagSpec(size), maxHours, attempt);
}

/** A country as a tooltip row shows it: its name and flag as recorded at an hour.
 * `known` is absent while pending; false when the name is missing or its read
 * failed. `nameFailed` distinguishes a failed read from a completed missing name. */
export type CountryLabel = (tag: string) => { name: string; flag?: string; known?: boolean; nameFailed?: boolean; flagPending?: boolean; flagReason?: string };

/** Countries named and flagged as the recording showed them at their hours (such as
 * a trade partner at the read behind a tooltip), never a later identity. Only the
 * countries `wanted` are asked for; until their names arrive, or where the
 * recording has none, its title says unavailable. Callers with
 * more distinct drawn hours can pass their own `maxHours`. `revision`
 * changes as names and flags arrive, for a chart to redraw. */
export function useCountryLabelsAt(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean,
  maxHours = MAX_NAME_HOURS) {
  const names = useCountryNamesRead(client, track, wanted, active, maxHours);
  const flags = useCountryFactsRead(client, track, wanted, active, flagReadSpec('panel'), maxHours);
  const labelAt = useCallback((hour: number): CountryLabel => tag => {
    const name = names.get(nameKey(hour, tag));
    const flag = flags.get(nameKey(hour, tag));
    return { name: typeof name === 'string' ? name : 'Country name unavailable', ...flagReadMetadata(flag),
      known: name === undefined ? undefined : typeof name === 'string', nameFailed: name === FAILED_FACT };
  }, [names, flags]);
  return { labelAt, revision: names.size + flags.size };
}
