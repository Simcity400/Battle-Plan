import type { TableProjection } from './model.ts';

const projections = new Map<string, TableProjection<any>>();

export function registerTableProjection<TOptions>(projection: TableProjection<TOptions>): TableProjection<TOptions> {
  projections.set(projection.kind, projection as TableProjection<any>);
  return projection;
}

export function tableProjection(kind: string): TableProjection<any> | undefined {
  return projections.get(kind);
}

export function registeredTableKinds(): string[] {
  return [...projections.keys()];
}
