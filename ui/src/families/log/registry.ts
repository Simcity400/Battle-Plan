import type { LogProjection } from './declaration.ts';

const projections = new Map<string, LogProjection<unknown>>();

export function registerLogProjection<TOptions>(projection: LogProjection<TOptions>): void {
  projections.set(projection.kind, projection as LogProjection<unknown>);
}

export function logProjection(kind: string): LogProjection<unknown> | undefined {
  return projections.get(kind);
}

export function registeredLogProjectionKinds(): readonly string[] {
  return [...projections.keys()];
}
