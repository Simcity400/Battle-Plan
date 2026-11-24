import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { Failure } from '../../dashboard/queryError';
import type { ArtResolution } from '../plan.ts';
import type {
  CountryFactsReply,
  CountryFlagChoice,
  CountryFlagImage,
  CountryGroupReply,
  CountryIdentitiesReply,
  CountryStyle,
  HourSpan,
  RecordingGap,
} from '../../generated/dashboard';

export type HourInterval = readonly [firstHour: number, lastHour: number];

export interface IdentityScope {
  readonly sourceId: string;
  readonly revision: string;
  readonly branchId: string;
  readonly content: number;
  readonly language?: string;
}

export type IdentityKind = 'country' | 'faction' | 'leader' | 'design' | 'state' | 'province' | 'region';

export type CountryIdentityKey =
  | { readonly kind: 'country'; readonly by: 'tag'; readonly tag: string }
  | { readonly kind: 'country'; readonly by: 'slot'; readonly slot: number }
  | { readonly kind: 'country'; readonly by: 'holder'; readonly slot: number; readonly countryId: number };

export interface FactionIdentityKey {
  readonly kind: 'faction';
  readonly id: string;
}

export interface LeaderIdentityKey {
  readonly kind: 'leader';
  readonly owner: string;
  readonly id: string;
}

export interface DesignIdentityKey {
  readonly kind: 'design';
  readonly owner: string;
  readonly variant: string;
}

export interface StateIdentityKey {
  readonly kind: 'state';
  readonly id: number | string;
}

export interface ProvinceIdentityKey {
  readonly kind: 'province';
  readonly id: number | string;
}

export interface RegionIdentityKey {
  readonly kind: 'region';
  readonly id: number | string;
}

export type EntityIdentityKey =
  | LeaderIdentityKey
  | DesignIdentityKey
  | StateIdentityKey
  | ProvinceIdentityKey
  | RegionIdentityKey;

export type IdentityKey =
  | CountryIdentityKey
  | FactionIdentityKey
  | LeaderIdentityKey
  | DesignIdentityKey
  | StateIdentityKey
  | ProvinceIdentityKey
  | RegionIdentityKey;

export type IdentityField<T> =
  | { readonly kind: 'known'; readonly value: T }
  | { readonly kind: 'missing'; readonly reason: string }
  | { readonly kind: 'pending' }
  | { readonly kind: 'failed'; readonly failure: Failure };

export interface CountryHolder {
  readonly slot: number;
  readonly countryId: number;
  readonly tag: string;
}

export type CountryIdentityResult =
  | {
      readonly kind: 'known';
      readonly valid: HourInterval;
      readonly nextBoundary: number;
      readonly holder: IdentityField<CountryHolder>;
      readonly tag: IdentityField<string>;
      readonly name: IdentityField<string>;
      readonly color: IdentityField<readonly [number, number, number]>;
      readonly flagChoice: IdentityField<CountryFlagChoice>;
      readonly warnings: readonly string[];
    }
  | {
      readonly kind: 'absent';
      readonly valid: HourInterval;
      readonly nextBoundary: number;
      readonly reason: string;
    }
  | { readonly kind: 'pending' }
  | { readonly kind: 'failed'; readonly failure: Failure };

export type FactionIdentityResult =
  | {
      readonly kind: 'known';
      readonly valid: HourInterval;
      readonly nextBoundary: number;
      readonly id: string;
      readonly recordedName: string;
      readonly name: IdentityField<string>;
      readonly leader: IdentityField<string>;
      readonly color: IdentityField<readonly [number, number, number]>;
      readonly incomplete: boolean;
      readonly warnings: readonly string[];
    }
  | {
      readonly kind: 'absent';
      readonly valid: HourInterval;
      readonly nextBoundary: number;
      readonly reason: string;
    }
  | { readonly kind: 'pending' }
  | { readonly kind: 'failed'; readonly failure: Failure };

export type EntityIdentityResult =
  | {
      readonly kind: 'known';
      readonly entityKind: 'leader' | 'design' | 'state' | 'province' | 'region';
      readonly valid: HourInterval;
      readonly nextBoundary: number;
      readonly holder: IdentityField<string>;
      readonly name: IdentityField<string>;
      readonly subtitle?: IdentityField<string>;
      readonly artKey?: IdentityField<string>;
      readonly warnings: readonly string[];
    }
  | {
      readonly kind: 'absent';
      readonly valid: HourInterval;
      readonly nextBoundary: number;
      readonly reason: string;
    }
  | { readonly kind: 'pending' }
  | { readonly kind: 'failed'; readonly failure: Failure };

export type IdentityResult<K extends IdentityKey = IdentityKey> =
  K extends CountryIdentityKey ? CountryIdentityResult
  : K extends FactionIdentityKey ? FactionIdentityResult
  : EntityIdentityResult;

export interface StyleIntervalEntry {
  readonly firstHour: number;
  readonly lastHour: number;
  readonly name: string | null;
  readonly color: readonly [number, number, number] | null;
  readonly absent: boolean;
  readonly reason: string | null;
  readonly flagChoice?: CountryFlagChoice | null;
  readonly warnings: readonly string[];
}

export interface EntityRunEntry {
  readonly firstHour: number;
  readonly lastHour: number;
  readonly absent?: boolean;
  readonly reason?: string;
  readonly holderReason?: string;
  readonly name: string | null;
  readonly nameReason?: string;
  readonly subtitle?: string | null;
  readonly subtitleReason?: string;
  readonly artKey?: string | null;
  readonly artReason?: string;
  readonly warnings?: readonly string[];
}

export interface DrawnFlagEntry {
  readonly url?: string;
  readonly image?: CountryFlagImage;
  readonly resolution?: ArtResolution;
  readonly failed?: Failure;
}

export interface SnapshotFailure {
  readonly message: string;
  readonly refused: boolean;
}

export interface SnapshotFailures {
  readonly facts?: SnapshotFailure;
  readonly identities?: SnapshotFailure;
  readonly groups?: SnapshotFailure;
  readonly styles?: Readonly<Record<string, SnapshotFailure>>;
  readonly entities?: Readonly<Record<string, SnapshotFailure>>;
}

export interface IdentitySnapshot {
  readonly scope: IdentityScope;
  readonly branchBounds?: HourInterval;
  readonly facts?: CountryFactsReply;
  readonly factsFinal?: boolean;
  readonly identities?: CountryIdentitiesReply;
  readonly groups?: CountryGroupReply;
  readonly styles?: Readonly<Record<string, readonly StyleIntervalEntry[]>>;
  readonly entities?: Readonly<Record<string, readonly EntityRunEntry[]>>;
  readonly entityUnknown?: Readonly<Partial<Record<EntityIdentityKey['kind'], readonly RecordingGap[]>>>;
  readonly drawnFlags?: Readonly<Record<string, DrawnFlagEntry>>;
  readonly failures?: SnapshotFailures;
}

export type { HourSpan };

export interface IdentityAsk {
  readonly key: IdentityKey;
  readonly hour: number;
}

export const countryTagKey = (tag: string): CountryIdentityKey => ({ kind: 'country', by: 'tag', tag });
export const countrySlotKey = (slot: number): CountryIdentityKey => ({ kind: 'country', by: 'slot', slot });
export const countryHolderKey = (slot: number, countryId: number): CountryIdentityKey => ({ kind: 'country', by: 'holder', slot, countryId });
export const factionKey = (id: string): FactionIdentityKey => ({ kind: 'faction', id });
export const leaderKey = (owner: string, id: string): LeaderIdentityKey => ({ kind: 'leader', owner, id });
export const designKey = (owner: string, variant: string): DesignIdentityKey => ({ kind: 'design', owner, variant });
export const stateKey = (id: number | string): StateIdentityKey => ({ kind: 'state', id });
export const provinceKey = (id: number | string): ProvinceIdentityKey => ({ kind: 'province', id });
export const regionKey = (id: number | string): RegionIdentityKey => ({ kind: 'region', id });

export function countryKeyFromString(raw: string): CountryIdentityKey {
  const country = countryFromKey(raw);
  if (typeof country === 'number') return countrySlotKey(country);
  if (raw.startsWith('holder:')) {
    const parts = raw.slice(7).split(':');
    if (parts.length === 2) {
      const slot = Number(parts[0]);
      const countryId = Number(parts[1]);
      if (Number.isInteger(slot) && slot >= 0 && Number.isInteger(countryId) && countryId >= 0) {
        return countryHolderKey(slot, countryId);
      }
    }
  }
  return countryTagKey(raw);
}

export function identityKeyString(key: IdentityKey): string {
  switch (key.kind) {
    case 'country':
      if (key.by === 'tag') return `country:tag:${key.tag}`;
      if (key.by === 'slot') return `country:slot:${key.slot}`;
      return `country:holder:${key.slot}:${key.countryId}`;
    case 'faction':
      return `faction:${key.id}`;
    case 'leader':
      return `leader:${key.owner}\u0000${key.id}`;
    case 'design':
      return `design:${key.owner}\u0000${key.variant}`;
    case 'state':
      return `state:${key.id}`;
    case 'province':
      return `province:${key.id}`;
    case 'region':
      return `region:${key.id}`;
  }
}

export function identityScopeKey(scope: IdentityScope): string {
  return JSON.stringify([scope.sourceId, scope.revision, scope.branchId, scope.content, scope.language ?? '']);
}

export function flagChoiceKey(choice: CountryFlagChoice): string {
  const searched = choice.searched?.some(Boolean) ? choice.searched.map(atlas => atlas === true) : null;
  return JSON.stringify(searched ? [choice.keys, choice.reasons, searched] : [choice.keys, choice.reasons]);
}

export function parseFlagChoiceKey(raw: string): CountryFlagChoice | null {
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (!Array.isArray(parsed) || parsed.length < 2 || parsed.length > 3) return null;
    const [keys, reasons, searched] = parsed as unknown[];
    if (!Array.isArray(keys) || keys.length !== 3 || !Array.isArray(reasons) || reasons.length !== 3) return null;
    if (searched !== undefined && (!Array.isArray(searched) || searched.length !== 3)) return null;
    const atlas = (value: unknown) => typeof value === 'string' ? value : null;
    const found = (value: unknown) => value === true ? true : null;
    return {
      keys: [atlas(keys[0]), atlas(keys[1]), atlas(keys[2])],
      reasons: [atlas(reasons[0]), atlas(reasons[1]), atlas(reasons[2])],
      ...(Array.isArray(searched) ? { searched: [found(searched[0]), found(searched[1]), found(searched[2])] } : {}),
    };
  } catch {
    return null;
  }
}

export function styleEntryFromReply(style: CountryStyle, hour: number, warnings: readonly string[]): StyleIntervalEntry {
  const valid = style.valid;
  const firstHour = valid && Number.isInteger(valid[0]) ? valid[0] : hour;
  const lastHour = valid && Number.isInteger(valid[1]) && valid[1] >= firstHour ? valid[1] : hour;
  return {
    firstHour,
    lastHour,
    name: style.name,
    color: style.color,
    absent: style.absent === true,
    reason: style.reason ?? null,
    flagChoice: style.flagChoice,
    warnings,
  };
}
