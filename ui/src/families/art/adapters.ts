import type { Canvas2D } from '../../charts/textWidth.ts';
import { drawArtSlot, drawFlagSlot, type ArtSlotState } from '../../draw/kit.ts';
import { classifyFailure } from '../../dashboard/queryError.ts';
import type { SpriteImage } from '../../generated/dashboard';
import type { FamilyTheme } from '../contract.ts';
import type { TooltipListItem } from '../host/tooltips.ts';
import type { DrawnFlagEntry } from '../identity/types.ts';
import type { LaneArtCache } from './laneCache.ts';
import {
  CURRENT_INSTALL_PROVENANCE,
  DEFAULT_ART_ORIGIN,
  NO_ART,
  PENDING_ART,
  defaultArtFrame,
  failedArt,
  productArt,
  readyArt,
  unavailableArt,
  refusedArt,
  type ArtOrigin,
  type ArtProvenance,
  type ArtResolution,
  type ArtResult,
} from './types.ts';

const flagEntryArtCache = new WeakMap<DrawnFlagEntry, ArtResolution>();

export function flagEntryToArt(entry: DrawnFlagEntry | undefined): ArtResolution {
  if (!entry) return PENDING_ART;
  const cached = flagEntryArtCache.get(entry);
  if (cached !== undefined) return cached;
  let result: ArtResolution;
  if (entry.failed) {
    const failure = classifyFailure(entry.failed);
    result = failure?.kind === 'refused' ? refusedArt(failure.reason, 'flag-read') : failedArt(entry.failed.message, true);
  } else if (entry.resolution) {
    result = entry.resolution;
  } else if (entry.image?.resolution) {
    result = productArt(entry.image.resolution);
  } else if (entry.url) {
    result = readyArt(entry.url, {
      assetId: entry.image?.key ?? entry.url,
      file: entry.image?.asset ?? '',
      frame: entry.image ? defaultArtFrame(entry.image.width, entry.image.height) : defaultArtFrame(),
    });
  } else {
    result = failedArt('The flag entry carries neither art nor a resolution', false);
  }
  flagEntryArtCache.set(entry, result);
  return result;
}

export function spriteImageToArt(image: SpriteImage): ArtResolution {
  if (image.resolution) return productArt(image.resolution);
  if (image.icon) {
    return readyArt(image.icon, {
      assetId: `${image.key}#${image.frame}`,
      frame: defaultArtFrame(0, 0, image.frame, 1),
    });
  }
  return failedArt('The sprite reply carried neither art nor a resolution', false);
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
  return unavailableArt(options.reason ?? 'The product gave no artwork address');
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
  label?: string,
): ArtSlotState {
  switch (state.status) {
    case 'noArtExpected':
      return { kind: 'none' };
    case 'pending':
      return { kind: 'pending' };
    case 'ready': {
      if (!state.bitmap) {
        const hasOut = state.frame.outputWidth > 0 && state.frame.outputHeight > 0;
        const width = hasOut ? state.frame.outputWidth : state.frame.frameWidth;
        const height = hasOut ? state.frame.outputHeight : state.frame.frameHeight;
        return width > 0 && height > 0 ? { kind: 'pending', width, height } : { kind: 'pending' };
      }
      const { width, height } = bitmapDimensions(state.bitmap, state.frame.outputWidth, state.frame.outputHeight);
      return { kind: 'ready', bitmap: state.bitmap, width, height };
    }
    case 'confirmedMissing':
      return { kind: 'missing', reason: state.reason, ...(label ? { label } : {}) };
    case 'unavailable':
      return { kind: 'unrecorded', reason: state.reason };
    case 'refused':
      return { kind: 'refused', reason: state.reason };
    case 'failed':
      return { kind: 'failed', reason: state.reason, retryable: state.retryable };
  }
}

export function artTooltipItem(label: string, art: ArtResult | undefined): TooltipListItem | null {
  const text = art ? artHoverText(art) : null;
  return text ? { label, missingReason: text } : null;
}

export const ART_MISSING_FROM_INSTALL = 'art missing from the current installation';

export function artHoverText(state: ArtResult): string | null {
  switch (state.status) {
    case 'noArtExpected':
    case 'ready':
      return null;
    case 'pending':
      return 'Loading';
    case 'confirmedMissing':
      return `${state.reason}; ${ART_MISSING_FROM_INSTALL}`;
    case 'unavailable':
      return state.reason;
    case 'refused':
      return `Refused: ${state.reason}`;
    case 'failed':
      return `Failed: ${state.reason}${state.retryable ? '; will retry' : ''}`;
  }
}

export interface MediaSlotSpec {
  readonly kind: 'flag' | 'art' | 'none';
  readonly art: ArtResult;
  readonly tag?: string;
  readonly name?: string;
  readonly resolvedName?: string;
  readonly dimmed?: boolean;
}

export interface MediaSlotDrawOptions {
  readonly artCache?: LaneArtCache;
  readonly radius?: number;
}

export function resolveCachedArt(art: ArtResult, artCache: LaneArtCache | undefined): ArtResult {
  return art.status === 'ready' && !art.bitmap && artCache ? artCache.resolve(art) : art;
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
  const resolved = resolveCachedArt(media.art, options.artCache);
  if (media.kind === 'flag') {
    drawFlagSlot(ctx, x, y, toArtSlotState(resolved, media.tag), theme, {
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
