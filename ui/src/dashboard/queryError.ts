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

export function queryFailure(error: QueryError | string): Failure {
  if (typeof error === 'string') return new ShownError(plainFailure(error).text, error);
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

export interface PlainFailure { text: string; detail?: string }

const productLead = (text: string) => /^\p{Lu}/u.test(text) && /\s/u.test(text) && !/[\\/]|\.\p{L}/u.test(text);

const productSentence = (text: string) => productLead(text) && /[.!?]$/u.test(text) && !/[:()[\]]/u.test(text);

export function plainFailure(message: string): PlainFailure {
  let depth = 0;
  for (let at = 0; at < message.length - 1; at++) {
    const char = message[at];
    if (char === '(' || char === '[') depth++;
    else if ((char === ')' || char === ']') && depth > 0) depth--;
    else if (char === ':' && message[at + 1] === ' ' && depth === 0 && at > 0) {
      const head = message.slice(0, at).trimEnd().replace(/\.+$/u, '');
      return productLead(head) ? { text: `${head}.`, detail: message } : { text: message };
    }
  }
  return { text: message };
}

export function actionFailure(title: string, raw: string): PlainFailure {
  const message = raw.trim();
  const plain = plainFailure(message);
  if (plain.detail) return plain;
  if (productSentence(message)) return { text: message };
  return { text: `${title.trim().replace(/\.+$/u, '')}.`, detail: raw };
}

export function failureKey(error: Failure): string {
  return JSON.stringify(failureWire(error));
}


export function failureDetails(values: Iterable<unknown>): string | undefined {
  const messages = new Set<string>();
  for (const value of values) if (value instanceof Error) messages.add(value.message);
  return messages.size ? [...messages].join('\n') : undefined;
}
