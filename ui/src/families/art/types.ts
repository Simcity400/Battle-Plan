export type ArtOriginKind = 'current-installation' | 'mod' | 'dlc' | 'base-game' | 'recorded';

export interface ArtOrigin {
  readonly kind: ArtOriginKind;
  readonly label: string;
  readonly currentInstallation: boolean;
}

export const CURRENT_INSTALLATION_ORIGIN: ArtOrigin = Object.freeze({
  kind: 'current-installation',
  label: 'Current installation',
  currentInstallation: true,
});

export interface ArtFrame {
  readonly index: number;
  readonly count: number;
  readonly x?: number;
  readonly y?: number;
  readonly width?: number;
  readonly height?: number;
}

export type ArtResolution =
  | { readonly kind: 'none' }
  | { readonly kind: 'pending' }
  | {
      readonly kind: 'ready';
      readonly asset: string;
      readonly origin: ArtOrigin;
      readonly frame?: ArtFrame;
      readonly width?: number;
      readonly height?: number;
    }
  | { readonly kind: 'missing'; readonly reason: string; readonly origin?: ArtOrigin }
  | { readonly kind: 'refused'; readonly reason: string }
  | { readonly kind: 'failed'; readonly reason: string; readonly retryable: boolean };

export interface ArtBitmap {
  readonly width: number;
  readonly height: number;
  close(): void;
}

export type DecodedArtState<TBitmap = ImageBitmap> =
  | { readonly kind: 'none' }
  | { readonly kind: 'pending' }
  | {
      readonly kind: 'ready';
      readonly bitmap: TBitmap;
      readonly origin: ArtOrigin;
      readonly frame?: ArtFrame;
      readonly width: number;
      readonly height: number;
    }
  | { readonly kind: 'missing'; readonly reason: string; readonly origin?: ArtOrigin }
  | { readonly kind: 'refused'; readonly reason: string }
  | {
      readonly kind: 'failed';
      readonly reason: string;
      readonly retryable: boolean;
      readonly retry: () => void;
    };

export const NO_ART: ArtResolution = Object.freeze({ kind: 'none' });
export const PENDING_ART: ArtResolution = Object.freeze({ kind: 'pending' });

export function readyArt(
  asset: string,
  origin: ArtOrigin = CURRENT_INSTALLATION_ORIGIN,
  extra?: { readonly frame?: ArtFrame; readonly width?: number; readonly height?: number },
): Extract<ArtResolution, { kind: 'ready' }> {
  return {
    kind: 'ready',
    asset,
    origin,
    ...(extra?.frame ? { frame: extra.frame } : {}),
    ...(extra?.width !== undefined ? { width: extra.width } : {}),
    ...(extra?.height !== undefined ? { height: extra.height } : {}),
  };
}

export function missingArt(reason: string, origin?: ArtOrigin): Extract<ArtResolution, { kind: 'missing' }> {
  return origin ? { kind: 'missing', reason, origin } : { kind: 'missing', reason };
}

export function refusedArt(reason: string): Extract<ArtResolution, { kind: 'refused' }> {
  return { kind: 'refused', reason };
}

export function failedArt(reason: string, retryable = true): Extract<ArtResolution, { kind: 'failed' }> {
  return { kind: 'failed', reason, retryable };
}
