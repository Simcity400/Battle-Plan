import { flagArtHandle } from '../art/provider.ts';
import type { ArtFlagSize, ArtHandle, BoundedReadView, EntityKind, FieldStatus } from '../plan.ts';
import { knownValue, notRecorded, type Value } from '../value.ts';
import type { TableCell, TableCellFlag } from './model.ts';

export interface IdentityBoundary {
  next: number;
}

export function newIdentityBoundary(): IdentityBoundary {
  return { next: Infinity };
}

export interface ArtState {
  readonly icon?: string;
  readonly iconPending?: boolean;
}

export function artState(readView: BoundedReadView, handle: ArtHandle | null): ArtState {
  if (!handle) return {};
  const art = readView.art(handle);
  if (art.status === 'ready') return { icon: art.url };
  if (art.status === 'pending') return { iconPending: true };
  return {};
}

export function urlArtHandle(url: string | null | undefined): ArtHandle | null {
  return url ? { kind: 'url', url } : null;
}

function flagState(readView: BoundedReadView, choice: FieldStatus<string> | undefined, size: ArtFlagSize): ArtState {
  if (!choice) return {};
  if (choice.status === 'ready') return artState(readView, flagArtHandle(choice.value, size));
  if (choice.status === 'pending') return { iconPending: true };
  return {};
}

export interface IdentityOptions {
  readonly atHour?: number;
  readonly flag?: ArtFlagSize;
  readonly boundary?: IdentityBoundary;
}

export interface ResolvedIdentity extends ArtState {
  readonly value: Value<string>;
  readonly name: string;
  readonly color?: string;
}

export function resolveIdentity(readView: BoundedReadView, kind: EntityKind, key: string, options: IdentityOptions = {}): ResolvedIdentity {
  const atHour = options.atHour ?? readView.hour;
  const who = readView.who(kind, key, atHour);
  if (options.boundary && atHour >= readView.hour) {
    options.boundary.next = Math.min(options.boundary.next, who.nextChangeAt);
  }
  const value: Value<string> = who.name.status === 'ready' && who.name.value.trim()
    ? knownValue(who.name.value)
    : who.name.status === 'pending'
      ? knownValue(key)
      : notRecorded(who.name.status === 'ready' ? 'Name not recorded' : who.name.reason);
  return {
    value,
    name: value.kind === 'known' ? value.value : key,
    ...flagState(readView, who.flagChoice, options.flag ?? 'panel'),
    ...(who.colour?.status === 'ready' ? { color: who.colour.value } : {}),
  };
}

export function identityCell(readView: BoundedReadView, kind: EntityKind, key: string | null, options: IdentityOptions = {}): TableCell {
  if (key === null) return { value: notRecorded('Not recorded'), iconSlot: true };
  const who = resolveIdentity(readView, kind, key, options);
  return {
    value: who.value,
    iconSlot: true,
    ...(who.icon ? { icon: who.icon } : {}),
    ...(who.iconPending ? { iconPending: true } : {}),
    ...(who.color ? { countryColor: who.color } : {}),
    ...(kind === 'country' ? { country: key } : {}),
  };
}

export interface IdentityFlagsOptions extends IdentityOptions {
  readonly dimmed?: readonly boolean[];
}

export function identityFlags(readView: BoundedReadView, kind: EntityKind, keys: readonly string[], options: IdentityFlagsOptions = {}): TableCellFlag[] {
  return keys.map((key, index) => {
    const who = resolveIdentity(readView, kind, key, options);
    return {
      tag: key,
      ...(who.icon ? { icon: who.icon } : {}),
      ...(who.iconPending ? { pending: true } : {}),
      ...(options.dimmed?.[index] ? { dimmed: true } : {}),
    };
  });
}

export function artNameCell(
  readView: BoundedReadView,
  name: string | null,
  handle: ArtHandle | null,
  missing: string,
  shape: TableCell['artShape'] = 'square',
): TableCell {
  return {
    value: name ? knownValue(name) : notRecorded(missing),
    iconSlot: true,
    artShape: shape,
    ...(name ? { artName: name } : {}),
    ...artState(readView, handle),
  };
}

export function artKindCell(readView: BoundedReadView, handle: ArtHandle | null, shape: TableCell['artShape'], name: string | null): TableCell {
  return {
    value: name ? knownValue(name) : notRecorded('Not recorded'),
    iconSlot: true,
    artShape: shape,
    ...(name ? { artName: name } : {}),
    ...artState(readView, handle),
  };
}

export function identityName(readView: BoundedReadView, kind: EntityKind, key: string, atHour?: number): string {
  return resolveIdentity(readView, kind, key, atHour === undefined ? {} : { atHour }).name;
}
