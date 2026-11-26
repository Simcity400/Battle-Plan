import { authoredHex } from '../../charts/palette.ts';
import type { DashboardSource } from '../../dashboard/client';
import { classifyFailure, type Failure } from '../../dashboard/queryError.ts';
import type {
  DisplayedHour,
  EntityKind,
  FieldStatus,
  IdentityProvider,
  IdentityResult as PlanIdentityResult,
} from '../plan.ts';
import type { IdentityBroker } from './broker.ts';
import type { IdentityIndex } from './index.ts';
import {
  flagChoiceKey,
  identityKeyString,
  parseEntityKey,
  type HourInterval,
  type IdentityResult,
  type IdentityAsk,
  type IdentityField,
  type IdentityScope,
} from './types.ts';

function failedFieldStatus(failure: Failure): FieldStatus<never> {
  return {
    status: 'failed',
    reason: failure.message,
    retryable: classifyFailure(failure)?.kind !== 'refused',
  };
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
      return failedFieldStatus(field.failure);
  }
}

export { parseEntityKey } from './types.ts';

const presented = new WeakMap<IdentityResult, PlanIdentityResult>();
const colours = new WeakMap<IdentityField<readonly [number, number, number]>, FieldStatus<string>>();

function colourStatus(field: IdentityField<readonly [number, number, number]>): FieldStatus<string> {
  let held = colours.get(field);
  if (!held) {
    held = toFieldStatus(field, authoredHex);
    colours.set(field, held);
  }
  return held;
}

export function identityResultFromIndex(
  index: IdentityIndex,
  kind: EntityKind,
  key: string,
  atHour: DisplayedHour,
): PlanIdentityResult {
  const raw = index.resolveText(kind, key, atHour);
  const prior = presented.get(raw);
  const point = raw.kind === 'pending' || raw.kind === 'failed';
  if (prior && prior.key === key && prior.kind === kind && (!point || Object.is(prior.validInterval[0], atHour))) {
    return prior;
  }

  const validInterval: HourInterval = point ? [atHour, atHour] : raw.valid;
  const nextChangeAt = point ? Infinity : raw.nextBoundary;
  let result: PlanIdentityResult;
  if (raw.kind !== 'known') {
    const field: FieldStatus<string> = raw.kind === 'absent' ? { status: 'absent', reason: raw.reason }
      : raw.kind === 'failed' ? failedFieldStatus(raw.failure) : { status: 'pending' };
    result = {
      kind, key, holder: field, name: field, validInterval, nextChangeAt,
      ...(kind === 'country' ? { tag: field, flagChoice: field } : {}),
      ...(kind === 'country' || kind === 'faction' ? { colour: field } : {}),
    };
  } else if ('tag' in raw) {
    result = {
      kind, key, validInterval, nextChangeAt,
      holder: toFieldStatus(raw.holder, held => held.tag),
      tag: toFieldStatus(raw.tag),
      name: toFieldStatus(raw.name),
      colour: colourStatus(raw.color),
      flagChoice: toFieldStatus(raw.flagChoice, flagChoiceKey),
    };
  } else if ('leader' in raw) {
    result = {
      kind, key, validInterval, nextChangeAt,
      holder: toFieldStatus(raw.leader),
      name: toFieldStatus(raw.name),
      colour: colourStatus(raw.color),
    };
  } else {
    result = {
      kind, key, validInterval, nextChangeAt,
      holder: toFieldStatus(raw.holder, () => key),
      name: toFieldStatus(raw.name),
    };
  }
  presented.set(raw, result);
  return result;
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
    demandOwner: 'view',
    resolve(kind: EntityKind, key: string, atHour: DisplayedHour): PlanIdentityResult {
      return identityResultFromIndex(index, kind, key, atHour);
    },
  };
}

export interface BrokerIdentityProviderOptions {
  readonly recording?: DashboardSource;
  readonly branchBounds?: HourInterval;
  readonly needFlags?: boolean;
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
    const asks = [...demanded.values()];
    demanded.clear();
    broker.setDemand(
      owner,
      scope,
      {
        asks,
        needFlags: options.needFlags ?? true,
        flagSize: 'panel',
      },
      options.recording,
      options.branchBounds,
    );
  };

  return {
    demandOwner: 'view',
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
      }
      return result;
    },
    clearDemands(): void {
      demanded.clear();
      broker.setDemand(owner, scope, null);
    },
  };
}
