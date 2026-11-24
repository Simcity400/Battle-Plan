import {
  DEFAULT_VIEW,
  ensureBoundProjection,
  type BoundProjectionKey,
  familyProjection,
  registerFamilyProjection,
  registeredFamilyProjections,
} from '../registry.ts';
import type { TableModel, TableProjection } from './model.ts';

export function registerTableProjection<TOptions>(
  projection: TableProjection<TOptions>,
  view = DEFAULT_VIEW,
): TableProjection<TOptions> {
  return registerFamilyProjection(projection, view) as TableProjection<TOptions>;
}

export function tableProjection(kind: string, view?: string): TableProjection<any> | undefined {
  return familyProjection<TableModel, any>('table', kind, view) as TableProjection<any> | undefined;
}

export function ensureTableProjection(
  binding: BoundProjectionKey,
  onReady: () => void,
): TableProjection<any> | 'loading' | 'unknown' {
  return ensureBoundProjection<TableModel, any>(binding, onReady) as TableProjection<any> | 'loading' | 'unknown';
}

export function registeredTableKinds(): string[] {
  return [...new Set(registeredFamilyProjections('table').map(entry => entry.kind))];
}
