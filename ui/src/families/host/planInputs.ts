import type { Failure } from '../../dashboard/queryError.ts';
import type { FamilyBodyStatus } from '../plan.ts';

export type PlanInput =
  | { readonly name: string; readonly state: 'ready' | 'pending'; readonly loadingLabel?: string }
  | { readonly name: string; readonly state: 'failed'; readonly failure: Failure; readonly retry: () => void };

interface InputState<T> { readonly data?: T; readonly error?: Failure; readonly retry: () => void }

export function inputOf<T>(name: string, read: InputState<T>, loadingLabel?: string): PlanInput {
  if (read.data !== undefined) return { name, state: 'ready' };
  return read.error ? { name, state: 'failed', failure: read.error, retry: read.retry } : { name, state: 'pending', loadingLabel };
}

export const allReady = (inputs: readonly PlanInput[]): boolean => inputs.every(input => input.state === 'ready');

function inputsStatus(inputs: readonly PlanInput[]): FamilyBodyStatus | null {
  const failed = inputs.find(input => input.state === 'failed');
  if (failed?.state === 'failed') return { kind: 'failed', failure: failed.failure, origin: 'local', onRetry: failed.retry };
  const pending = inputs.find(input => input.state === 'pending');
  return pending?.state === 'pending' ? { kind: 'loading', label: pending.loadingLabel ?? `Reading ${pending.name}…` } : null;
}

export interface BodyFacts {
  readonly early: FamilyBodyStatus | null;
  readonly plan: readonly PlanInput[];
  readonly outcome: FamilyBodyStatus | null;
  readonly tiles: readonly PlanInput[];
  readonly units: readonly PlanInput[];
  readonly settled: FamilyBodyStatus;
}

export function familyBodyStatus(facts: BodyFacts): FamilyBodyStatus {
  return facts.early ?? inputsStatus(facts.plan) ?? facts.outcome ?? inputsStatus([...facts.tiles, ...facts.units]) ?? facts.settled;
}
