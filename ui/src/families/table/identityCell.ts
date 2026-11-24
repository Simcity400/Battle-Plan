import { flagArtHandle } from '../art/provider.ts';
import { artHandleKey } from '../art/types.ts';
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
  readonly iconArt?: string;
}

export function artState(readView: BoundedReadView, handle: ArtHandle | null): ArtState {
  if (!handle) return {};
  const art = readView.art(handle);
  const iconArt = artHandleKey(handle);
  if (art.status === 'ready') return { icon: art.url, iconArt };
  if (art.status === 'pending') return { iconPending: true, iconArt };
  return { iconArt };
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

export function identitySignature(who: ResolvedIdentity): string {
  const reason = who.value.kind === 'notRecorded' ? who.value.reason : '';
  return `${who.value.kind}\u0000${who.name}\u0000${reason}\u0000${who.icon ?? ''}\u0000${who.iconPending ? 1 : 0}\u0000${who.iconArt ?? ''}\u0000${who.color ?? ''}`;
}

export type IdentityRead = readonly [tag: string, atHour: number, signature: string];

export interface IdentityAsker {
  readonly ask: (tag: string, atHour: number) => ResolvedIdentity;
  readonly reads: IdentityRead[];
}

export function identityAsker(readView: BoundedReadView, boundary: IdentityBoundary): IdentityAsker {
  const reads: IdentityRead[] = [];
  const held = new Map<string, ResolvedIdentity>();
  return {
    reads,
    ask(tag, atHour) {
      const at = Math.min(atHour, readView.hour);
      const id = `${tag}\u0000${at}`;
      let who = held.get(id);
      if (!who) {
        who = resolveIdentity(readView, 'country', tag, { atHour: at, boundary });
        held.set(id, who);
        reads.push([tag, at, identitySignature(who)]);
      }
      return who;
    },
  };
}

export function identityMoved(reads: readonly IdentityRead[], readView: BoundedReadView): boolean {
  for (let i = 0; i < reads.length; i++) {
    const [tag, atHour, signature] = reads[i]!;
    if (identitySignature(resolveIdentity(readView, 'country', tag, { atHour })) !== signature) return true;
  }
  return false;
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
