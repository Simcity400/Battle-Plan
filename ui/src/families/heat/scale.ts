import { mix } from '../../shell/appearance/color.ts';
import { authoredHex, colour, readableColour } from '../../charts/palette.ts';
import type { ArtHandle, BoundedReadView, EntityKind } from '../contract.ts';
import { flagArtHandle } from '../art/provider.ts';

export interface HeatPart {
  readonly id: string;
  readonly label: string;
  readonly color: string;
  readonly share: number;
  readonly text?: string;
}

export function signedRampColour(
  value: number,
  low: number,
  high: number,
  grey: string,
  negative: string,
  positive: string,
): string {
  if (!Number.isFinite(value) || value === 0) return grey;
  const extent = value < 0 ? Math.abs(Math.min(0, low)) : Math.max(0, high);
  if (!(extent > 0)) return grey;
  return mix(grey, value < 0 ? negative : positive, Math.min(1, Math.abs(value) / extent));
}

export function sequentialRampColour(
  value: number,
  highest: number,
  grey: string,
  accent: string,
): string {
  if (!Number.isFinite(value) || value <= 0 || !(highest > 0)) return grey;
  return mix(grey, accent, Math.min(1, value / highest));
}

export function marginColour(parts: readonly HeatPart[], grey: string): string | null {
  let first: HeatPart | undefined;
  let second = 0;
  for (const part of parts) {
    if (part.share <= 0) continue;
    if (!first || part.share > first.share || (part.share === first.share && part.id.localeCompare(first.id) < 0)) {
      second = first?.share ?? 0;
      first = part;
    } else {
      second = Math.max(second, part.share);
    }
  }
  if (!first) return null;
  return mix(grey, first.color, Math.max(0, Math.min(1, (first.share - second) / 0.5)));
}

export function splitStripes(
  parts: readonly HeatPart[],
  top: number,
  height: number,
): readonly { readonly color: string; readonly top: number; readonly height: number }[] {
  let used = 0;
  const out: { readonly color: string; readonly top: number; readonly height: number }[] = [];
  for (const part of parts) {
    const share = Math.min(Math.max(0, part.share), Math.max(0, 1 - used));
    if (share <= 0) continue;
    out.push({ color: part.color, top: top + used * height, height: share * height });
    used += share;
  }
  return out;
}

export function resolveOrdinalColour(surface: string, ordinal: number): string {
  return colour(surface, ordinal);
}

export function resolveRecordedColour(
  rgb: readonly [number, number, number] | null | undefined,
  surface: string,
  fallbackOrdinal: number,
): string {
  const hex = rgb ? authoredHex(rgb) : null;
  return hex ? readableColour(hex, surface) : colour(surface, fallbackOrdinal);
}

export interface ResolvedEntityPresentation {
  readonly label: string;
  readonly nameMissing?: string;
  readonly nameFailed?: { readonly reason: string; readonly retryable: boolean };
  readonly pending?: boolean;
  readonly color?: string;
  readonly flag?: ArtHandle | null;
}

export function resolveEntityFromReadView(
  readView: BoundedReadView,
  kind: EntityKind,
  key: string,
  atHour: number | undefined,
  fallbackLabel: string,
  fallbackMissing?: string | null,
  surface?: string,
): ResolvedEntityPresentation {
  const identity = readView.who(kind, key, atHour);
  let label = fallbackLabel;
  let nameMissing: string | undefined = fallbackMissing ?? undefined;
  let nameFailed: { readonly reason: string; readonly retryable: boolean } | undefined;
  let pending = false;

  if (identity.name.status === 'ready') {
    label = identity.name.value;
    nameMissing = undefined;
  } else if (identity.name.status === 'pending') {
    pending = true;
  } else if (identity.name.status === 'absent') {
    nameMissing = identity.name.reason;
  } else if (identity.name.status === 'failed') {
    nameFailed = { reason: identity.name.reason, retryable: identity.name.retryable };
    nameMissing = undefined;
  }

  let color: string | undefined;
  if (identity.colour?.status === 'ready') {
    color = surface ? readableColour(identity.colour.value, surface) : identity.colour.value;
  }

  let flag: ArtHandle | null | undefined;
  if (kind === 'country') {
    if (identity.flagChoice?.status === 'ready') {
      flag = flagArtHandle(identity.flagChoice.value, 'panel');
    } else {
      flag = null;
    }
  }

  return {
    label,
    ...(nameMissing ? { nameMissing } : {}),
    ...(nameFailed ? { nameFailed } : {}),
    ...(pending ? { pending: true } : {}),
    ...(color ? { color } : {}),
    ...(flag !== undefined ? { flag } : {}),
  };
}
