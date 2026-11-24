import { flagArtHandle } from '../art/provider.ts';
import type { CountryLook } from '../looks.ts';
import type { ArtFlagSize, ArtHandle, BoundedReadView, EntityKind, FieldStatus } from '../plan.ts';
import { knownValue, notRecorded } from '../value.ts';
import type { TableCell } from './model.ts';

const TIMELESS = Number.MIN_SAFE_INTEGER;

export interface IdentityBoundary {
  next: number;
}

export function newIdentityBoundary(): IdentityBoundary {
  return { next: Infinity };
}

type ArtLook = Pick<CountryLook, 'flag' | 'flagPending'>;

export function artLook(readView: BoundedReadView, handle: ArtHandle | null): ArtLook {
  if (!handle) return {};
  const art = readView.art(handle);
  if (art.status === 'ready') return { flag: art.url };
  if (art.status === 'pending') return { flagPending: true };
  return {};
}

function flagLook(readView: BoundedReadView, choice: FieldStatus<string> | undefined, size: ArtFlagSize): ArtLook {
  if (!choice) return {};
  if (choice.status === 'ready') return artLook(readView, flagArtHandle(choice.value, size));
  if (choice.status === 'pending') return { flagPending: true };
  return {};
}

function lookCell(look: CountryLook, missing: string, country?: string): TableCell {
  return {
    value: look.name ? knownValue(look.name) : notRecorded(look.status ?? missing),
    iconSlot: true,
    looks: [look],
    lookHour: TIMELESS,
    ...(country ? { country } : {}),
  };
}

export interface IdentityCellOptions {
  readonly atHour?: number;
  readonly flag?: ArtFlagSize;
  readonly boundary?: IdentityBoundary;
}

export function identityCell(readView: BoundedReadView, kind: EntityKind, key: string | null, options: IdentityCellOptions = {}): TableCell {
  if (key === null) return { value: notRecorded('Not recorded'), iconSlot: true };
  const atHour = options.atHour ?? readView.hour;
  const who = readView.who(kind, key, atHour);
  if (options.boundary && atHour >= readView.hour) {
    options.boundary.next = Math.min(options.boundary.next, who.nextChangeAt);
  }
  const name = who.name.status === 'ready' ? who.name.value : who.name.status === 'pending' ? key : null;
  const look: CountryLook = {
    from: TIMELESS,
    name,
    ...flagLook(readView, who.flagChoice, options.flag ?? 'small'),
    ...(who.colour?.status === 'ready' ? { color: who.colour.value } : {}),
    ...(who.name.status === 'absent' || who.name.status === 'failed' ? { status: who.name.reason } : {}),
  };
  return lookCell(look, 'Name not recorded', kind === 'country' ? key : undefined);
}

export function artNameCell(readView: BoundedReadView, name: string | null, handle: ArtHandle | null, missing: string): TableCell {
  return lookCell({ from: TIMELESS, name, ...artLook(readView, handle), ...(name ? {} : { status: missing }) }, missing);
}

export function identityName(readView: BoundedReadView, kind: EntityKind, key: string, atHour?: number): string {
  const who = readView.who(kind, key, atHour ?? readView.hour);
  return who.name.status === 'ready' ? who.name.value : key;
}
