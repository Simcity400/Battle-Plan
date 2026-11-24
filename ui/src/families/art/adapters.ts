import type { CountryFlagImage } from '../../generated/dashboard';
import type { DrawnFlagEntry } from '../identity/types.ts';
import {
  CURRENT_INSTALLATION_ORIGIN,
  NO_ART,
  PENDING_ART,
  failedArt,
  missingArt,
  readyArt,
  type ArtOrigin,
  type ArtResolution,
} from './types.ts';

export function flagEntryToArt(entry: DrawnFlagEntry | undefined, origin: ArtOrigin = CURRENT_INSTALLATION_ORIGIN): ArtResolution {
  if (!entry) return PENDING_ART;
  if (entry.failed) return failedArt(entry.failed.message, true);
  if (entry.url) {
    return readyArt(entry.url, origin, entry.image ? { width: entry.image.width, height: entry.image.height } : undefined);
  }
  return missingArt(entry.reason ?? 'Flag unavailable', origin);
}

export function flagImageToArt(
  image: CountryFlagImage | undefined,
  options: { readonly pending?: boolean; readonly reason?: string | null; readonly origin?: ArtOrigin } = {},
): ArtResolution {
  const origin = options.origin ?? CURRENT_INSTALLATION_ORIGIN;
  if (image) return readyArt(image.png, origin, { width: image.width, height: image.height });
  if (options.pending) return PENDING_ART;
  return missingArt(options.reason ?? 'Flag unavailable', origin);
}

export function urlToArt(
  url: string | null | undefined,
  options: {
    readonly expected?: boolean;
    readonly pending?: boolean;
    readonly reason?: string | null;
    readonly failed?: string | null;
    readonly origin?: ArtOrigin;
  } = {},
): ArtResolution {
  if (options.expected === false) return NO_ART;
  if (options.failed) return failedArt(options.failed, true);
  if (url) return readyArt(url, options.origin ?? CURRENT_INSTALLATION_ORIGIN);
  if (options.pending) return PENDING_ART;
  return missingArt(options.reason ?? 'Art not found in the current installation', options.origin ?? CURRENT_INSTALLATION_ORIGIN);
}
