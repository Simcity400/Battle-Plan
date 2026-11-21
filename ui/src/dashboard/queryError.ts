import type { QueryError } from '../generated/dashboard';

export class RefusedError extends Error {
  readonly text: string;
  constructor(text: string) {
    super(text);
    this.name = 'RefusedError';
    this.text = text;
  }
}

export class ShownError extends Error {
  readonly text: string;
  constructor(text: string, detail: string) {
    super(detail);
    this.name = 'ShownError';
    this.text = text;
  }
}

export type Failure = Error;

export function queryFailure(error: QueryError): Failure {
  switch (error.kind) {
    case 'refused': return new RefusedError(error.text);
    case 'failed': return new Error(error.message);
    default: return new Error('Invalid dashboard error reply');
  }
}

export function asFailure(error: unknown): Failure {
  return error instanceof Error ? error : new Error(String(error));
}

export function failureText(error: Failure | null | undefined, generic: string): string {
  return error instanceof RefusedError || error instanceof ShownError ? error.text : generic;
}

export function failureWire(error: Failure): QueryError {
  return error instanceof RefusedError ? { kind: 'refused', text: error.text }
    : { kind: 'failed', message: error instanceof ShownError ? `${error.text}
${error.message}` : error.message };
}

export function failureKey(error: Failure): string {
  return JSON.stringify(failureWire(error));
}


export function failureDetails(values: Iterable<unknown>): string | undefined {
  const messages = new Set<string>();
  for (const value of values) if (value instanceof Error) messages.add(value.message);
  return messages.size ? [...messages].join('\n') : undefined;
}
