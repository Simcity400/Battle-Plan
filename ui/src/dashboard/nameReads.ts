import { failureText, type Failure } from './queryError.ts';

export type NameRead =
  | { kind: 'absent' }
  | { kind: 'loading' }
  | { kind: 'value'; value: string }
  | { kind: 'failed'; failure: Failure };

export const ABSENT_NAME: NameRead = { kind: 'absent' };
export const LOADING_NAME: NameRead = { kind: 'loading' };

export function nameReadFrom(value: string | null | undefined | Failure, loading: boolean): NameRead {
  if (value instanceof Error) return { kind: 'failed', failure: value };
  if (typeof value === 'string') return { kind: 'value', value };
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
  return { name: nameReadText(read), known: read?.kind === 'loading' ? undefined : read?.kind === 'value',
    nameFailed: failure !== undefined, ...(failure ? { nameFailure: failure, nameDetail: failure.message } : {}) };
}

export function sameNameRead(before: NameRead | undefined, after: NameRead): boolean {
  if (!before || before.kind !== after.kind) return false;
  if (before.kind === 'value' && after.kind === 'value') return before.value === after.value;
  if (before.kind === 'failed' && after.kind === 'failed') return before.failure === after.failure;
  return true;
}
