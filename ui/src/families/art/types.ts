import type { ArtFrame, ArtHandle, ArtOrigin, ArtProvenance, ArtResolution, ArtResult, ProductArtResolution } from '../plan.ts';

export type { ArtOrigin, ArtProvenance, ArtResolution, ArtResult, ProductArtResolution };

export function productArt(resolution: ProductArtResolution): ArtResolution {
  return resolution as ArtResolution;
}

export interface ArtBitmap {
  readonly width: number;
  readonly height: number;
  close(): void;
}

export const DEFAULT_ART_ORIGIN: ArtOrigin = Object.freeze({ kind: 'baseGame' });

export const CURRENT_INSTALL_PROVENANCE: ArtProvenance = Object.freeze({
  fromCurrentInstall: true,
  label: 'Current installation',
  scopeKey: 'current',
});

export function defaultArtFrame(width = 0, height = 0, frameIndex = 0, frameCount = 1): ArtFrame {
  return {
    frameIndex,
    frameCount,
    frameWidth: width,
    frameHeight: height,
    textureWidth: width * frameCount,
    textureHeight: height,
    outputWidth: width,
    outputHeight: height,
  };
}

export function flagArtKey(size: string, choiceKey: string): string {
  return `flag\u0000${size}\u0000${choiceKey}`;
}

export function artHandleKey(handle: ArtHandle): string {
  switch (handle.kind) {
    case 'flag':
      return flagArtKey(handle.size, handle.choice);
    case 'sprite':
      return `sprite\u0000${handle.frame ?? 0}\u0000${handle.text === true ? 1 : 0}\u0000${handle.recorded !== undefined ? `1\u0000${handle.recorded.length}:${handle.recorded}` : '0'}\u0000${handle.key}`;
    case 'portrait':
      return `portrait\u0000${handle.hour}\u0000${handle.leader}`;
    case 'technology':
      return `technology\u0000${handle.databaseIndex}\u0000${handle.token}`;
    case 'path':
      return `path\u0000${handle.frame ?? 0}\u0000${handle.frames ?? 1}\u0000${handle.path}`;
    case 'url':
      return `url\u0000${handle.frame ?? 0}\u0000${handle.url}`;
  }
}

export const NO_ART: ArtResolution = Object.freeze({ status: 'noArtExpected' });
export const PENDING_ART: ArtResolution = Object.freeze({ status: 'pending' });

export function readyArt(
  url: string,
  options: {
    readonly assetId?: string;
    readonly origin?: ArtOrigin;
    readonly file?: string;
    readonly frame?: ArtFrame;
    readonly provenance?: ArtProvenance;
  } = {},
): Extract<ArtResolution, { status: 'ready' }> {
  return {
    status: 'ready',
    assetId: options.assetId ?? url,
    url,
    origin: options.origin ?? DEFAULT_ART_ORIGIN,
    file: options.file ?? '',
    frame: options.frame ?? defaultArtFrame(),
    provenance: options.provenance ?? CURRENT_INSTALL_PROVENANCE,
  };
}

export function unavailableArt(reason: string): Extract<ArtResolution, { status: 'unavailable' }> {
  return { status: 'unavailable', reason };
}

export function refusedArt(
  reason: string,
  construct = 'unsupported',
  options: { readonly origin?: ArtOrigin; readonly file?: string } = {},
): Extract<ArtResolution, { status: 'refused' }> {
  return {
    status: 'refused',
    reason,
    construct,
    ...(options.origin !== undefined ? { origin: options.origin } : {}),
    ...(options.file !== undefined ? { file: options.file } : {}),
  };
}

export function failedArt(reason: string, retryable = true): Extract<ArtResolution, { status: 'failed' }> {
  return { status: 'failed', reason, retryable };
}
