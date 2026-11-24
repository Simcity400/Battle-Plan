import { mix } from '../../shell/appearance/color.ts';
import { readableColour, reservedGrey } from '../../charts/palette.ts';
import type { BoundedReadView, EntityKind } from '../contract.ts';
import { flagChoiceArt } from '../art/provider.ts';
import type { ArtResult } from '../art/types.ts';
import { LOADING_NAME } from '../table/identityCell.ts';

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

export interface ResolvedEntityPresentation {
  readonly label: string;
  readonly nameMissing?: string;
  readonly nameFailed?: { readonly reason: string; readonly retryable: boolean };
  readonly pending?: boolean;
  readonly color?: string;
  readonly flag?: ArtResult;
  readonly validFrom: number;
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
    label = LOADING_NAME;
    pending = true;
  } else if (identity.name.status === 'absent') {
    nameMissing = identity.name.reason;
  } else if (identity.name.status === 'failed') {
    nameFailed = { reason: identity.name.reason, retryable: identity.name.retryable };
    nameMissing = undefined;
  }

  let color: string | undefined;
  if (identity.colour?.status === 'ready') {
    color = surface ? readableOn(identity.colour.value, surface) : identity.colour.value;
  }

  const flag = kind === 'country' ? flagChoiceArt(readView, identity.flagChoice, 'panel') : undefined;

  return {
    label,
    ...(nameMissing ? { nameMissing } : {}),
    ...(nameFailed ? { nameFailed } : {}),
    ...(pending ? { pending: true } : {}),
    ...(color ? { color } : {}),
    ...(flag !== undefined ? { flag } : {}),
    validFrom: identity.validInterval[0],
  };
}

export function labelsAtReadings(
  readView: BoundedReadView,
  entities: readonly { readonly tag?: string | null; readonly label: string; readonly validFrom: number }[],
  readHours: Iterable<number>,
): (index: number, hour: number) => string {
  const hours = [...new Set(readHours)].sort((a, b) => a - b);
  const earlier = new Map<string, string>();
  entities.forEach((entity, index) => {
    if (!entity.tag) return;
    for (const hour of hours) {
      if (hour >= entity.validFrom) break;
      earlier.set(`${index}@${hour}`, resolveEntityFromReadView(readView, 'country', entity.tag, hour, entity.label).label);
    }
  });
  return (index, hour) => {
    const entity = entities[index];
    if (!entity) return '';
    return hour >= entity.validFrom ? entity.label : earlier.get(`${index}@${hour}`) ?? entity.label;
  };
}

export interface DatedColour {
  readonly colour: string;
  readonly through: number;
}

const HELD_COLOURS = 4096;
const readable = new Map<string, string>();
const greys = new Map<string, string>();

function held(cache: Map<string, string>, key: string, make: () => string): string {
  let made = cache.get(key);
  if (made === undefined) {
    if (cache.size >= HELD_COLOURS) cache.clear();
    cache.set(key, made = make());
  }
  return made;
}

export function readableOn(colour: string, surface: string, minContrast?: number): string {
  return held(readable, `${colour}|${surface}|${minContrast ?? ''}`, () => readableColour(colour, surface, minContrast));
}

export function contestedGrey(surface: string, identities: readonly string[]): string {
  return held(greys, `${surface}|${identities.join(',')}`, () => reservedGrey(surface, identities));
}

export function countryColourAt(readView: BoundedReadView, key: string, hour: number, surface: string, fallback: string): DatedColour {
  const identity = readView.who('country', key, hour);
  const colour = identity.colour?.status === 'ready' ? readableOn(identity.colour.value, surface) : fallback;
  return { colour, through: identity.validInterval[1] };
}

export interface RegionLabel {
  readonly label: string;
  readonly nameMissing?: string;
}

export function regionLabel(readView: BoundedReadView, id: string | number, pendingLabel: string | null): RegionLabel {
  const name = readView.who('region', String(id)).name;
  if (name.status === 'ready') return { label: name.value };
  if (name.status === 'absent' || name.status === 'failed') return { label: 'Region name not recorded', nameMissing: name.reason };
  return pendingLabel ? { label: pendingLabel } : { label: 'Region name not recorded', nameMissing: 'Region name not read yet' };
}
