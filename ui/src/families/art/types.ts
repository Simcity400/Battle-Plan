import type { ArtFrame, ArtHandle, ArtOrigin, ArtProvenance, ArtResolution, ArtResult } from '../plan.ts';

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

export function artHandleKey(handle: ArtHandle): string {
  switch (handle.kind) {
    case 'flag':
      return JSON.stringify(['flag', handle.choice, handle.size]);
    case 'sprite':
      return JSON.stringify(['sprite', handle.key, handle.frame ?? 0, handle.recorded ?? null, handle.text === true]);
    case 'portrait':
      return JSON.stringify(['portrait', handle.leader, handle.hour]);
    case 'path':
      return JSON.stringify(['path', handle.path, handle.frame ?? 0, handle.frames ?? 1]);
    case 'url':
      return JSON.stringify(['url', handle.url, handle.frame ?? 0]);
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

export function missingArt(
  reason: string,
  options: { readonly origin?: ArtOrigin; readonly file?: string } = {},
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
