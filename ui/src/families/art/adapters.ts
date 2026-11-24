import type { ArtSlotState, FlagSlotState } from '../../draw/kit.ts';
import type { CountryFlagImage } from '../../generated/dashboard';
import type { DrawnFlagEntry } from '../identity/types.ts';
import {
  CURRENT_INSTALL_PROVENANCE,
  DEFAULT_ART_ORIGIN,
  NO_ART,
  PENDING_ART,
  defaultArtFrame,
  failedArt,
  missingArt,
  readyArt,
  type ArtOrigin,
  type ArtProvenance,
  type ArtResolution,
  type ArtResult,
} from './types.ts';

type FlagImageWithResolution = CountryFlagImage & { readonly resolution?: ArtResolution | null };

export function flagEntryToArt(
  entry: DrawnFlagEntry | undefined,
  provenance: ArtProvenance = CURRENT_INSTALL_PROVENANCE,
): ArtResolution {
  if (!entry) return PENDING_ART;
  if (entry.failed) return failedArt(entry.failed.message, true);
  const withRes = entry.image as FlagImageWithResolution | undefined;
  if (withRes?.resolution) return withRes.resolution;
  if (entry.url) {
    return readyArt(entry.url, {
      assetId: entry.image?.key ?? entry.url,
      file: entry.image?.asset ?? '',
      frame: entry.image ? defaultArtFrame(entry.image.width, entry.image.height) : defaultArtFrame(),
      provenance,
    });
  }
  return missingArt(entry.reason ?? 'Flag unavailable');
}

export function flagImageToArt(
  image: CountryFlagImage | undefined,
  options: { readonly pending?: boolean; readonly reason?: string | null; readonly provenance?: ArtProvenance } = {},
): ArtResolution {
  const withRes = image as FlagImageWithResolution | undefined;
  if (withRes?.resolution) return withRes.resolution;
  if (image) {
    return readyArt(image.png, {
      assetId: image.key,
      file: image.asset,
      frame: defaultArtFrame(image.width, image.height),
      provenance: options.provenance ?? CURRENT_INSTALL_PROVENANCE,
    });
  }
  if (options.pending) return PENDING_ART;
  return missingArt(options.reason ?? 'Flag unavailable');
}

export function urlToArt(
  url: string | null | undefined,
  options: {
    readonly expected?: boolean;
    readonly pending?: boolean;
    readonly reason?: string | null;
    readonly failed?: string | null;
    readonly origin?: ArtOrigin;
    readonly provenance?: ArtProvenance;
  } = {},
): ArtResolution {
  if (options.expected === false) return NO_ART;
  if (options.failed) return failedArt(options.failed, true);
  if (url) {
    return readyArt(url, {
      origin: options.origin ?? DEFAULT_ART_ORIGIN,
      provenance: options.provenance ?? CURRENT_INSTALL_PROVENANCE,
    });
  }
  if (options.pending) return PENDING_ART;
  return missingArt(options.reason ?? 'Art not found in the current installation');
}

function bitmapDimensions(bitmap: CanvasImageSource, frameWidth: number, frameHeight: number): { readonly width: number; readonly height: number } {
  if (frameWidth > 0 && frameHeight > 0) return { width: frameWidth, height: frameHeight };
  const sized = bitmap as { readonly naturalWidth?: number; readonly naturalHeight?: number; readonly width?: number; readonly height?: number };
  return {
    width: sized.naturalWidth ?? sized.width ?? 0,
    height: sized.naturalHeight ?? sized.height ?? 0,
  };
}

export function toArtSlotState<TBitmap extends CanvasImageSource>(
  state: ArtResult<TBitmap>,
  resolvedName?: string,
): ArtSlotState {
  switch (state.status) {
    case 'noArtExpected':
      return { kind: 'none' };
    case 'pending':
      return { kind: 'pending' };
    case 'ready': {
      if (!state.bitmap) return { kind: 'pending' };
      const { width, height } = bitmapDimensions(state.bitmap, state.frame.outputWidth, state.frame.outputHeight);
      return { kind: 'ready', bitmap: state.bitmap, width, height };
    }
    case 'confirmedMissing':
      return { kind: 'missing', ...(resolvedName ? { resolvedName } : {}) };
    case 'refused':
      return { kind: 'refused', reason: state.reason };
    case 'failed':
      return { kind: 'failed', reason: state.reason };
  }
}

export function toFlagSlotState<TBitmap extends CanvasImageSource>(
  state: ArtResult<TBitmap>,
  tag?: string,
): FlagSlotState {
  switch (state.status) {
    case 'pending':
      return { kind: 'pending' };
    case 'ready': {
      if (!state.bitmap) return { kind: 'pending' };
      const { width, height } = bitmapDimensions(state.bitmap, state.frame.outputWidth, state.frame.outputHeight);
      return { kind: 'ready', bitmap: state.bitmap, width, height };
    }
    case 'failed':
      return { kind: 'failed' };
    case 'noArtExpected':
    case 'confirmedMissing':
    case 'refused':
      return { kind: 'missing', ...(tag ? { tag } : {}) };
  }
}
