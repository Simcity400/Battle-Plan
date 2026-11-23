import type { QueryError } from '../generated/dashboard';

export type NotInRecordingCause =
  | { kind: 'capture-mode'; mode?: string }
  | { kind: 'older-format' }
  | { kind: 'format-changed' }
  | { kind: 'mod'; modName: string; what: string }
  | { kind: 'dlc'; dlcName: string }
  | { kind: 'game-build'; version: string }
  | { kind: 'damage' };

function capitalise(text: string): string {
  return text ? text[0].toUpperCase() + text.slice(1) : text;
}

export function notInRecordingSentence(cause: NotInRecordingCause, subject?: string): string {
  const what = subject?.trim() || 'this data';
  switch (cause.kind) {
    case 'capture-mode':
      return cause.mode && cause.mode !== 'map'
        ? `This is a ${cause.mode} recording, which holds the campaign map and battle windows only.`
        : 'This is a Map recording, which holds the campaign map and battle windows only.';
    case 'older-format':
      return `This recording was made before BattlePlan recorded ${what}.`;
    case 'format-changed':
      return `BattlePlan changed how it records ${what} after this recording was made, so it isn’t shown.`;
    case 'mod':
      return `${cause.modName} changes ${cause.what} in a way BattlePlan can’t read yet.`;
    case 'dlc':
      return `This campaign was played without ${cause.dlcName}, which ${what} needs.`;
    case 'game-build':
      return `${capitalise(what)} couldn’t be checked against game version ${cause.version}, so it was withheld.`;
    case 'damage':
      return `Part of this recording is damaged, so ${what} is withheld.`;
  }
}

export class RefusedError extends Error {
  readonly text: string;
  readonly causeKind?: NotInRecordingCause;
  constructor(text: string, causeKind?: NotInRecordingCause) {
    super(text);
    this.name = 'RefusedError';
    this.text = text;
    this.causeKind = causeKind;
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

export type ClassifiedFailure =
  | { kind: 'refused'; cause?: NotInRecordingCause; reason: string; detail: string }
  | { kind: 'retryable'; reason?: string; detail: string };

export function classifyFailure(error: Failure | null | undefined): ClassifiedFailure | null {
  if (!error) return null;
  if (error instanceof RefusedError) {
    const plain = plainFailure(error.text);
    return { kind: 'refused', cause: error.causeKind, reason: plain.text, detail: error.message };
  }
  if (error instanceof ShownError) {
    return { kind: 'retryable', reason: error.text, detail: error.message };
  }
  return { kind: 'retryable', detail: error.message };
}

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
