import type { Failure } from '../../dashboard/queryError.ts';
import type { FamilyBodyStatus } from '../plan.ts';

export type PlanInput =
  | { readonly name: string; readonly state: 'ready' | 'pending' }
  | { readonly name: string; readonly state: 'failed'; readonly failure: Failure; readonly retry: () => void };

interface InputState<T> { readonly data?: T; readonly error?: Failure; readonly retry: () => void }

export function inputOf<T>(name: string, read: InputState<T>): PlanInput {
  if (read.data !== undefined) return { name, state: 'ready' };
  return read.error ? { name, state: 'failed', failure: read.error, retry: read.retry } : { name, state: 'pending' };
}

export const allReady = (inputs: readonly PlanInput[]): boolean => inputs.every(input => input.state === 'ready');

export function inputsStatus(inputs: readonly PlanInput[]): FamilyBodyStatus | null {
  const failed = inputs.find(input => input.state === 'failed');
  if (failed?.state === 'failed') return { kind: 'failed', failure: failed.failure, origin: 'local', onRetry: failed.retry };
  const pending = inputs.find(input => input.state === 'pending');
  return pending ? { kind: 'loading', label: `Reading ${pending.name}…` } : null;
}
