import type { DashboardSource } from '../../dashboard/client';
import type {
  DisplayedHour,
  EntityKind,
  FieldStatus,
  IdentityDemand,
  IdentityProvider,
  IdentityResult as PlanIdentityResult,
} from '../plan.ts';
import type { IdentityBroker } from './broker.ts';
import type { IdentityIndex } from './index.ts';
import {
  countryKeyFromString,
  designKey,
  factionKey,
  flagChoiceKey,
  identityKeyString,
  leaderKey,
  provinceKey,
  regionKey,
  stateKey,
  type CountryIdentityResult,
  type EntityIdentityResult,
  type FactionIdentityResult,
  type HourInterval,
  type IdentityAsk,
  type IdentityField,
  type IdentityKey,
  type IdentityScope,
} from './types.ts';

function rgbCss(rgb: readonly [number, number, number]): string {
  return `rgb(${rgb[0]}, ${rgb[1]}, ${rgb[2]})`;
}

function toFieldStatus<T, U = T>(field: IdentityField<T>, map?: (value: T) => U): FieldStatus<U> {
  switch (field.kind) {
    case 'known':
      return { status: 'ready', value: map ? map(field.value) : (field.value as unknown as U) };
    case 'pending':
      return { status: 'pending' };
    case 'missing':
      return { status: 'absent', reason: field.reason };
    case 'failed':
      return { status: 'absent', reason: field.failure.message };
  }
}

export function parseEntityKey(kind: EntityKind, key: string): IdentityKey {
  switch (kind) {
    case 'country':
      return countryKeyFromString(key);
    case 'faction':
      return factionKey(key);
    case 'leader': {
      const sep = key.indexOf(':');
      return sep >= 0 ? leaderKey(key.slice(0, sep), key.slice(sep + 1)) : leaderKey('', key);
    }
    case 'design': {
      const sep = key.indexOf(':');
      return sep >= 0 ? designKey(key.slice(0, sep), key.slice(sep + 1)) : designKey('', key);
    }
    case 'state': {
      const num = Number(key);
      return stateKey(Number.isSafeInteger(num) ? num : key);
    }
    case 'province': {
      const num = Number(key);
      return provinceKey(Number.isSafeInteger(num) ? num : key);
    }
    case 'region': {
      const num = Number(key);
      return regionKey(Number.isSafeInteger(num) ? num : key);
    }
  }
}

export function identityResultFromIndex(
  index: IdentityIndex,
  kind: EntityKind,
  key: string,
  atHour: DisplayedHour,
): PlanIdentityResult {
  const parsed = parseEntityKey(kind, key);
  const raw = index.resolve(parsed, atHour);

  if (parsed.kind === 'country') {
    const res = raw as CountryIdentityResult;
    if (res.kind === 'pending') {
      return {
        kind,
        key,
        atHour,
        holder: { status: 'pending' },
        tag: { status: 'pending' },
        name: { status: 'pending' },
        colour: { status: 'pending' },
        flagChoice: { status: 'pending' },
        validInterval: [atHour, atHour],
        nextChangeAt: Infinity,
      };
    }
    if (res.kind === 'absent') {
      const absent: FieldStatus<string> = { status: 'absent', reason: res.reason };
      return {
        kind,
        key,
        atHour,
        holder: absent,
        tag: absent,
        name: absent,
        colour: absent,
        flagChoice: absent,
        validInterval: res.valid,
        nextChangeAt: res.nextBoundary,
      };
    }
    if (res.kind === 'failed') {
      const failed: FieldStatus<string> = { status: 'absent', reason: res.failure.message };
      return {
        kind,
        key,
        atHour,
        holder: failed,
        tag: failed,
        name: failed,
        colour: failed,
        flagChoice: failed,
        validInterval: [atHour, atHour],
        nextChangeAt: Infinity,
      };
    }
    const holder: FieldStatus<string> = res.holder.kind === 'known'
      ? { status: 'ready', value: res.holder.value.tag }
      : parsed.by === 'tag' && res.tag.kind === 'known'
        ? { status: 'ready', value: res.tag.value }
        : toFieldStatus(res.holder, () => '');
    return {
      kind,
      key,
      atHour,
      holder,
      tag: toFieldStatus(res.tag),
      name: toFieldStatus(res.name),
      colour: toFieldStatus(res.color, rgbCss),
      flagChoice: toFieldStatus(res.flagChoice, flagChoiceKey),
      validInterval: res.valid,
      nextChangeAt: res.nextBoundary,
    };
  }

  if (parsed.kind === 'faction') {
    const res = raw as FactionIdentityResult;
    if (res.kind === 'pending') {
      return {
        kind,
        key,
        atHour,
        holder: { status: 'pending' },
        name: { status: 'pending' },
        colour: { status: 'pending' },
        validInterval: [atHour, atHour],
        nextChangeAt: Infinity,
      };
    }
    if (res.kind === 'absent') {
      const absent: FieldStatus<string> = { status: 'absent', reason: res.reason };
      return {
        kind,
        key,
        atHour,
        holder: absent,
        name: absent,
        colour: absent,
        validInterval: res.valid,
        nextChangeAt: res.nextBoundary,
      };
    }
    if (res.kind === 'failed') {
      const failed: FieldStatus<string> = { status: 'absent', reason: res.failure.message };
      return {
        kind,
        key,
        atHour,
        holder: failed,
        name: failed,
        colour: failed,
        validInterval: [atHour, atHour],
        nextChangeAt: Infinity,
      };
    }
    return {
      kind,
      key,
      atHour,
      holder: toFieldStatus(res.leader),
      name: toFieldStatus(res.name),
      colour: toFieldStatus(res.color, rgbCss),
      validInterval: res.valid,
      nextChangeAt: res.nextBoundary,
    };
  }

  const res = raw as EntityIdentityResult;
  if (res.kind === 'pending') {
    return {
      kind,
      key,
      atHour,
      holder: { status: 'pending' },
      name: { status: 'pending' },
      validInterval: [atHour, atHour],
      nextChangeAt: Infinity,
    };
  }
  if (res.kind === 'absent') {
    const absent: FieldStatus<string> = { status: 'absent', reason: res.reason };
    return {
      kind,
      key,
      atHour,
      holder: absent,
      name: absent,
      validInterval: res.valid,
      nextChangeAt: res.nextBoundary,
    };
  }
  if (res.kind === 'failed') {
    const failed: FieldStatus<string> = { status: 'absent', reason: res.failure.message };
    return {
      kind,
      key,
      atHour,
      holder: failed,
      name: failed,
      validInterval: [atHour, atHour],
      nextChangeAt: Infinity,
    };
  }
  return {
    kind,
    key,
    atHour,
    holder: { status: 'ready', value: key },
    name: toFieldStatus(res.name),
    validInterval: res.valid,
    nextChangeAt: res.nextBoundary,
  };
}

function hasPendingField(result: PlanIdentityResult): boolean {
  return result.holder.status === 'pending'
    || result.name.status === 'pending'
    || result.tag?.status === 'pending'
    || result.colour?.status === 'pending'
    || result.flagChoice?.status === 'pending';
}

export function createSnapshotIdentityProvider(index: IdentityIndex): IdentityProvider {
  return {
    resolve(kind: EntityKind, key: string, atHour: DisplayedHour): PlanIdentityResult {
      return identityResultFromIndex(index, kind, key, atHour);
    },
  };
}

export interface BrokerIdentityProviderOptions {
  readonly recording?: DashboardSource;
  readonly branchBounds?: HourInterval;
  readonly needFlags?: boolean;
  readonly onDemand?: IdentityDemand;
}

export function createBrokerIdentityProvider(
  broker: IdentityBroker,
  scope: IdentityScope,
  owner: object,
  options: BrokerIdentityProviderOptions = {},
): IdentityProvider & { readonly clearDemands: () => void } {
  const demanded = new Map<string, IdentityAsk>();
  let scheduled = false;

  const flushDemands = () => {
    scheduled = false;
    if (!demanded.size) return;
    broker.setDemand(
      owner,
      scope,
      {
        asks: [...demanded.values()],
        needFlags: options.needFlags ?? true,
        flagSize: 'panel',
      },
      options.recording,
      options.branchBounds,
    );
  };

  return {
    resolve(kind: EntityKind, key: string, atHour: DisplayedHour): PlanIdentityResult {
      const index = broker.indexOf(scope, options.recording, options.branchBounds);
      const result = identityResultFromIndex(index, kind, key, atHour);
      if (hasPendingField(result)) {
        const parsed = parseEntityKey(kind, key);
        const askKey = `${identityKeyString(parsed)}@${atHour}`;
        if (!demanded.has(askKey)) {
          demanded.set(askKey, { key: parsed, hour: atHour });
          if (!scheduled) {
            scheduled = true;
            queueMicrotask(flushDemands);
          }
        }
        options.onDemand?.(kind, key, atHour);
      }
      return result;
    },
    clearDemands(): void {
      demanded.clear();
      broker.setDemand(owner, scope, null);
    },
  };
}
