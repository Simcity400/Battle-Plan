import type { Canvas2D } from '../../charts/textWidth.ts';
import { drawArtSlot, drawFlagSlot, type ArtSlotState, type FlagSlotState } from '../../draw/kit.ts';
import type { CountryFlagImage, SpriteImage } from '../../generated/dashboard';
import type { FamilyTheme } from '../contract.ts';
import type { DrawnFlagEntry } from '../identity/types.ts';
import type { LaneArtCache } from './laneCache.ts';
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

export function flagEntryToArt(entry: DrawnFlagEntry | undefined): ArtResolution {
  if (!entry) return PENDING_ART;
  if (entry.failed) return failedArt(entry.failed.message, true);
  if (entry.resolution) return entry.resolution;
  if (entry.image?.resolution) return entry.image.resolution;
  if (entry.url) {
    return readyArt(entry.url, {
      assetId: entry.image?.key ?? entry.url,
      file: entry.image?.asset ?? '',
      frame: entry.image ? defaultArtFrame(entry.image.width, entry.image.height) : defaultArtFrame(),
    });
  }
  return missingArt(entry.reason ?? 'Flag unavailable');
}

export function flagImageToArt(
  image: CountryFlagImage | undefined,
  options: { readonly pending?: boolean; readonly reason?: string | null } = {},
): ArtResolution {
  if (image?.resolution) return image.resolution;
  if (image) {
    return readyArt(image.png, {
      assetId: image.key,
      file: image.asset,
      frame: defaultArtFrame(image.width, image.height),
    });
  }
  if (options.pending) return PENDING_ART;
  return missingArt(options.reason ?? 'Flag unavailable');
}

export function spriteImageToArt(
  image: SpriteImage | undefined,
  options: { readonly pending?: boolean; readonly reason?: string | null } = {},
): ArtResolution {
  if (image?.resolution) return image.resolution;
  if (image?.icon) {
    return readyArt(image.icon, {
      assetId: `${image.key}#${image.frame}`,
      frame: defaultArtFrame(0, 0, image.frame, 1),
    });
  }
  if (options.pending) return PENDING_ART;
  return missingArt(options.reason ?? 'Sprite art unavailable');
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
      return { kind: 'none' };
    case 'refused':
      return { kind: 'refused', reason: state.reason };
    case 'confirmedMissing':
      return { kind: 'missing', ...(tag ? { tag } : {}) };
  }
}

export interface MediaSlotSpec {
  readonly kind: 'flag' | 'art' | 'none';
  readonly art: ArtResult;
  readonly iconUrl?: string | null;
  readonly tag?: string;
  readonly name?: string;
  readonly resolvedName?: string;
  readonly dimmed?: boolean;
}

export interface MediaSlotDrawOptions {
  readonly artCache?: LaneArtCache;
  readonly icons?: ReadonlyMap<string, ImageBitmap | null>;
  readonly radius?: number;
}

export function resolveCachedArt(
  art: ArtResult,
  options?: { readonly artCache?: LaneArtCache; readonly icons?: ReadonlyMap<string, ImageBitmap | null>; readonly iconUrl?: string | null },
): ArtResult {
  if (art.status === 'ready' && !art.bitmap) {
    const cached = options?.artCache?.resolve(art);
    if (cached) return cached;
    const url = options?.iconUrl ?? art.url;
    const bmp = options?.icons?.get(url);
    if (bmp) return { ...art, bitmap: bmp };
  }
  return art;
}

export function drawMediaSlot(
  ctx: Canvas2D,
  x: number,
  middleY: number,
  width: number,
  height: number,
  media: MediaSlotSpec | undefined,
  theme: FamilyTheme,
  options: MediaSlotDrawOptions = {},
): void {
  if (!media || media.kind === 'none' || width <= 0 || height <= 0) return;
  const y = Math.round(middleY - height / 2);
  const resolved = resolveCachedArt(media.art, {
    artCache: options.artCache,
    icons: options.icons,
    iconUrl: media.iconUrl,
  });
  if (media.kind === 'flag') {
    drawFlagSlot(ctx, x, y, toFlagSlotState(resolved, media.tag), theme, {
      width,
      height,
      ...(media.dimmed ? { dimmed: true } : {}),
    });
    return;
  }
  drawArtSlot(ctx, x, y, width, height, toArtSlotState(resolved, media.resolvedName ?? media.name), theme, {
    radius: options.radius ?? 2,
  });
}
