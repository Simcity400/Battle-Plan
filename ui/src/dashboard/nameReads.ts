import { failureText, type Failure } from './queryError.ts';

export type NameRead =
  | { kind: 'absent'; reason?: string }
  | { kind: 'loading' }
  | { kind: 'value'; value: string }
  | { kind: 'failed'; failure: Failure };

export type NameFact = string | { readonly reason: string };

export const ABSENT_NAME: NameRead = { kind: 'absent' };
export const LOADING_NAME: NameRead = { kind: 'loading' };

export function nameReadFrom(value: NameFact | null | undefined | Failure, loading: boolean): NameRead {
  if (value instanceof Error) return { kind: 'failed', failure: value };
  if (typeof value === 'string') return { kind: 'value', value };
  if (value && typeof value === 'object' && typeof value.reason === 'string') return { kind: 'absent', reason: value.reason };
  return value === undefined && loading ? LOADING_NAME : ABSENT_NAME;
}

export function nameReadFailure(read: NameRead | undefined): Failure | undefined {
  return read?.kind === 'failed' ? read.failure : undefined;
}

export function nameReadText(read: NameRead | undefined, resolved?: string): string {
  if (read === undefined && resolved !== undefined) return resolved;
  switch (read?.kind) {
    case 'loading': return 'Loading name…';
    case 'value': return read.value;
    case 'failed': return failureText(read.failure, 'Couldn’t load country name.');
    default: return 'Country name unavailable';
  }
}

export function nameReadMetadata(read: NameRead | undefined) {
  const failure = nameReadFailure(read);
  const detail = failure ? failure.message : read?.kind === 'absent' ? read.reason : undefined;
  return { name: nameReadText(read), known: read?.kind === 'loading' ? undefined : read?.kind === 'value',
    nameFailed: failure !== undefined, ...(failure ? { nameFailure: failure } : {}), ...(detail ? { nameDetail: detail } : {}) };
}

export function sameNameRead(before: NameRead | undefined, after: NameRead): boolean {
  if (!before || before.kind !== after.kind) return false;
  if (before.kind === 'value' && after.kind === 'value') return before.value === after.value;
  if (before.kind === 'failed' && after.kind === 'failed') return before.failure === after.failure;
  if (before.kind === 'absent' && after.kind === 'absent') return before.reason === after.reason;
  return true;
}
