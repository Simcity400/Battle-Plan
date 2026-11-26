import { flagArtHandle, flagChoiceArt } from '../art/provider.ts';
import { artHandleKey, unavailableArt, type ArtResult } from '../art/types.ts';
import type { ArtFlagSize, ArtHandle, BoundedReadView, EntityKind, FieldStatus } from '../plan.ts';
import { knownValue, notRecorded, type Value } from '../value.ts';
import type { TableCell, TableCellFlag } from './model.ts';

export interface IdentityBoundary {
  next: number;
}

export function newIdentityBoundary(): IdentityBoundary {
  return { next: Infinity };
}

export interface CellArt {
  readonly art?: ArtResult;
  readonly artKey?: string;
}

export interface IdentityOptions {
  readonly atHour?: number;
  readonly flag?: ArtFlagSize;
  readonly boundary?: IdentityBoundary;
}

export interface ResolvedIdentity extends CellArt, ResolvedName {
  readonly color?: string;
}

export const NAME_LOADING = 'Name is loading';
export const LOADING_NAME = 'Loading name…';

const NOT_RECORDED_LOADING: Value<string> = Object.freeze({ kind: 'notRecorded', reason: NAME_LOADING });
const NOT_RECORDED_EMPTY_NAME: Value<string> = Object.freeze({ kind: 'notRecorded', reason: 'Name not recorded' });
const NOT_RECORDED_GENERIC: Value<string> = Object.freeze({ kind: 'notRecorded', reason: 'Not recorded' });
const NULL_IDENTITY_CELL: TableCell = Object.freeze({ value: NOT_RECORDED_GENERIC, iconSlot: true });

interface CachedFlagHandle {
  readonly handle: ArtHandle;
  readonly key: string;
}

const FLAG_HANDLE_CACHE_MAX = 2048;
const flagHandleCaches: Record<string, Map<string, CachedFlagHandle>> = Object.create(null);

function cachedFlagHandle(choice: string, size: ArtFlagSize): CachedFlagHandle {
  let byChoice = flagHandleCaches[size];
  if (!byChoice) flagHandleCaches[size] = byChoice = new Map();
  let held = byChoice.get(choice);
  if (held !== undefined) return held;
  if (byChoice.size >= FLAG_HANDLE_CACHE_MAX) byChoice.clear();
  const handle = flagArtHandle(choice, size);
  held = { handle, key: artHandleKey(handle) };
  byChoice.set(choice, held);
  return held;
}

const nameValueCache = new WeakMap<object, Value<string>>();

function valueOfNameField(field: FieldStatus<string>): Value<string> {
  if (field.status === 'pending') return NOT_RECORDED_LOADING;
  const cached = nameValueCache.get(field);
  if (cached !== undefined) return cached;
  let value: Value<string>;
  if (field.status === 'ready') {
    value = field.value.trim() ? knownValue(field.value) : NOT_RECORDED_EMPTY_NAME;
  } else {
    value = notRecorded(field.reason);
  }
  nameValueCache.set(field, value);
  return value;
}

export function cellArt(readView: BoundedReadView, handle: ArtHandle | null, missing: string): { readonly art: ArtResult; readonly artKey?: string } {
  return handle ? { art: readView.art(handle), artKey: artHandleKey(handle) } : { art: unavailableArt(missing) };
}

export interface ResolvedName {
  readonly value: Value<string>;
  readonly name: string;
  readonly loading: boolean;
}

export function resolveName(readView: BoundedReadView, kind: EntityKind, key: string, options?: Pick<IdentityOptions, 'atHour' | 'boundary'>): ResolvedName & { readonly who: ReturnType<BoundedReadView['who']> } {
  const atHour = options?.atHour ?? readView.hour;
  const boundary = options?.boundary;
  const who = readView.who(kind, key, atHour);
  if (boundary && atHour >= readView.hour && who.nextChangeAt < boundary.next) {
    boundary.next = who.nextChangeAt;
  }
  const loading = who.name.status === 'pending';
  const value = valueOfNameField(who.name);
  return { value, name: value.kind === 'known' ? value.value : loading ? LOADING_NAME : key, loading, who };
}

export function resolveCountryColorAt(
  readView: BoundedReadView,
  key: string,
  atHour: number,
  boundary?: IdentityBoundary,
): string | undefined {
  const who = readView.who('country', key, atHour);
  if (boundary && atHour >= readView.hour && who.nextChangeAt < boundary.next) {
    boundary.next = who.nextChangeAt;
  }
  return who.colour?.status === 'ready' ? who.colour.value : undefined;
}

export function resolveIdentityAt(
  readView: BoundedReadView,
  kind: EntityKind,
  key: string,
  atHour: number,
  flag: ArtFlagSize = 'panel',
  boundary?: IdentityBoundary,
): ResolvedIdentity {
  const who = readView.who(kind, key, atHour);
  if (boundary && atHour >= readView.hour && who.nextChangeAt < boundary.next) {
    boundary.next = who.nextChangeAt;
  }
  const loading = who.name.status === 'pending';
  const value = valueOfNameField(who.name);
  const result: {
    value: Value<string>;
    name: string;
    loading: boolean;
    art?: ArtResult;
    artKey?: string;
    color?: string;
  } = {
    value,
    name: value.kind === 'known' ? value.value : loading ? LOADING_NAME : key,
    loading,
  };
  if (kind === 'country') {
    const choice = who.flagChoice;
    if (choice?.status === 'ready') {
      const entry = cachedFlagHandle(choice.value, flag);
      result.art = readView.art(entry.handle);
      result.artKey = entry.key;
    } else {
      result.art = flagChoiceArt(readView, choice, flag);
    }
  }
  if (who.colour?.status === 'ready') result.color = who.colour.value;
  return result;
}

export function resolveIdentity(readView: BoundedReadView, kind: EntityKind, key: string, options?: IdentityOptions): ResolvedIdentity {
  return resolveIdentityAt(readView, kind, key, options?.atHour ?? readView.hour, options?.flag ?? 'panel', options?.boundary);
}

export function identityCellAt(
  readView: BoundedReadView,
  kind: EntityKind,
  key: string | null,
  atHour: number,
  flag: ArtFlagSize = 'panel',
  boundary?: IdentityBoundary,
): TableCell {
  if (key === null) return NULL_IDENTITY_CELL;
  const who = readView.who(kind, key, atHour);
  if (boundary && atHour >= readView.hour && who.nextChangeAt < boundary.next) {
    boundary.next = who.nextChangeAt;
  }
  const loading = who.name.status === 'pending';
  const cell: {
    value: Value<string>;
    iconSlot: true;
    loading?: true;
    art?: ArtResult;
    artKey?: string;
    countryColor?: string;
    country?: string;
  } = {
    value: valueOfNameField(who.name),
    iconSlot: true,
  };
  if (loading) cell.loading = true;
  if (kind === 'country') {
    const choice = who.flagChoice;
    if (choice?.status === 'ready') {
      const entry = cachedFlagHandle(choice.value, flag);
      cell.art = readView.art(entry.handle);
      cell.artKey = entry.key;
    } else {
      cell.art = flagChoiceArt(readView, choice, flag);
    }
    cell.country = key;
  }
  if (who.colour?.status === 'ready') cell.countryColor = who.colour.value;
  return cell;
}

export function identityCell(readView: BoundedReadView, kind: EntityKind, key: string | null, options?: IdentityOptions): TableCell {
  if (key === null) return NULL_IDENTITY_CELL;
  return identityCellAt(readView, kind, key, options?.atHour ?? readView.hour, options?.flag ?? 'panel', options?.boundary);
}

export interface CountryIdentitySnapshot {
  readonly tag: string;
  readonly nameKind: Value['kind'];
  readonly nameText: string;
  readonly color: string | undefined;
  readonly artKey: string | undefined;
  readonly artStatus: ArtResult['status'];
  readonly artDetail: string | undefined;
}

export function captureCountryIdentity(
  readView: BoundedReadView,
  tag: string,
  atHour: number,
  flag: ArtFlagSize = 'panel',
): CountryIdentitySnapshot {
  const who = readView.who('country', tag, atHour);
  const nameVal = valueOfNameField(who.name);
  const nameText = nameVal.kind === 'known' ? nameVal.value : (nameVal as { readonly reason: string }).reason;
  const color = who.colour?.status === 'ready' ? who.colour.value : undefined;
  const choice = who.flagChoice;
  let artKey: string | undefined;
  let artStatus: ArtResult['status'];
  let artDetail: string | undefined;
  if (choice?.status === 'ready') {
    const entry = cachedFlagHandle(choice.value, flag);
    artKey = entry.key;
    const art = readView.art(entry.handle);
    artStatus = art.status;
    artDetail = art.status === 'ready' ? art.url : ('reason' in art ? art.reason : undefined);
  } else if (!choice || choice.status === 'pending') {
    artStatus = 'pending';
  } else if (choice.status === 'absent') {
    artStatus = 'unavailable';
    artDetail = choice.reason;
  } else {
    artStatus = 'failed';
    artDetail = choice.reason;
  }
  return { tag, nameKind: nameVal.kind, nameText, color, artKey, artStatus, artDetail };
}

export function matchesCountryIdentity(
  readView: BoundedReadView,
  snapshot: CountryIdentitySnapshot,
  atHour: number,
  flag: ArtFlagSize = 'panel',
): boolean {
  const who = readView.who('country', snapshot.tag, atHour);
  const choice = who.flagChoice;
  let artKey: string | undefined;
  let artStatus: ArtResult['status'];
  let artDetail: string | undefined;
  if (choice?.status === 'ready') {
    const entry = cachedFlagHandle(choice.value, flag);
    artKey = entry.key;
    const art = readView.art(entry.handle);
    artStatus = art.status;
    artDetail = art.status === 'ready' ? art.url : ('reason' in art ? art.reason : undefined);
  } else if (!choice || choice.status === 'pending') {
    artStatus = 'pending';
  } else if (choice.status === 'absent') {
    artStatus = 'unavailable';
    artDetail = choice.reason;
  } else {
    artStatus = 'failed';
    artDetail = choice.reason;
  }
  if (artKey !== snapshot.artKey || artStatus !== snapshot.artStatus || artDetail !== snapshot.artDetail) return false;
  const color = who.colour?.status === 'ready' ? who.colour.value : undefined;
  if (color !== snapshot.color) return false;
  const nameVal = valueOfNameField(who.name);
  if (nameVal.kind !== snapshot.nameKind) return false;
  const nameText = nameVal.kind === 'known' ? nameVal.value : (nameVal as { readonly reason: string }).reason;
  return nameText === snapshot.nameText;
}

export interface IdentityFlagsOptions extends IdentityOptions {
  readonly dimmed?: readonly boolean[];
}

export function identityFlags(readView: BoundedReadView, kind: EntityKind, keys: readonly string[], options?: IdentityFlagsOptions): TableCellFlag[] {
  const atHour = options?.atHour ?? readView.hour;
  const flag = options?.flag ?? 'panel';
  const boundary = options?.boundary;
  const dimmed = options?.dimmed;
  const out = new Array<TableCellFlag>(keys.length);
  for (let index = 0; index < keys.length; index++) {
    const key = keys[index]!;
    const who = resolveIdentityAt(readView, kind, key, atHour, flag, boundary);
    const item: { tag: string; art: ArtResult; artKey?: string; dimmed?: true } = {
      tag: key,
      art: who.art ?? flagChoiceArt(readView, undefined),
    };
    if (who.artKey) item.artKey = who.artKey;
    if (dimmed?.[index]) item.dimmed = true;
    out[index] = item;
  }
  return out;
}

export function artNameCell(
  readView: BoundedReadView,
  name: string | null,
  handle: ArtHandle | null,
  missing: string,
  shape: TableCell['artShape'] = 'square',
  artMissing?: string,
): TableCell {
  const cell: {
    value: Value<string>;
    iconSlot: true;
    artShape: TableCell['artShape'];
    artName?: string;
    art: ArtResult;
    artKey?: string;
  } = {
    value: name ? knownValue(name) : notRecorded(missing),
    iconSlot: true,
    artShape: shape,
    art: handle ? readView.art(handle) : unavailableArt(artMissing ?? missing),
  };
  if (name) cell.artName = name;
  if (handle) cell.artKey = artHandleKey(handle);
  return cell;
}

export function artKindCell(readView: BoundedReadView, handle: ArtHandle | null, shape: TableCell['artShape'], name: string | null, missing: string): TableCell {
  const cell: {
    value: Value<string>;
    iconSlot: true;
    artShape: TableCell['artShape'];
    artName?: string;
    art: ArtResult;
    artKey?: string;
  } = {
    value: name ? knownValue(name) : NOT_RECORDED_GENERIC,
    iconSlot: true,
    artShape: shape,
    art: handle ? readView.art(handle) : unavailableArt(missing),
  };
  if (name) cell.artName = name;
  if (handle) cell.artKey = artHandleKey(handle);
  return cell;
}

export function identityName(readView: BoundedReadView, kind: EntityKind, key: string, atHour?: number): string {
  const who = readView.who(kind, key, atHour ?? readView.hour);
  const value = valueOfNameField(who.name);
  return value.kind === 'known' ? value.value : who.name.status === 'pending' ? LOADING_NAME : key;
}

export function identityNameOr(readView: BoundedReadView, kind: EntityKind, key: string, unnamed: string, atHour?: number): string {
  const who = readView.who(kind, key, atHour ?? readView.hour);
  const value = valueOfNameField(who.name);
  if (value.kind === 'known') return value.value;
  return who.name.status === 'pending' ? LOADING_NAME : unnamed;
}
