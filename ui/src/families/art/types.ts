import type { ArtFrame, ArtOrigin, ArtProvenance, ArtResolution, ArtResult } from '../plan.ts';

export type { ArtFrame, ArtOrigin, ArtProvenance, ArtResolution, ArtResult };

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

export function missingArt(
  reason: string,
  options: { readonly origin?: ArtOrigin | null; readonly file?: string | null } = {},
): Extract<ArtResolution, { status: 'confirmedMissing' }> {
  return {
    status: 'confirmedMissing',
    reason,
    ...(options.origin !== undefined ? { origin: options.origin } : {}),
    ...(options.file !== undefined ? { file: options.file } : {}),
  };
}

export function refusedArt(
  reason: string,
  construct = 'unsupported',
  options: { readonly origin?: ArtOrigin | null; readonly file?: string | null } = {},
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
