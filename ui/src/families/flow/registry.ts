import {
  DEFAULT_VIEW,
  ensureFamilyProjection,
  familyProjection,
  registerFamilyProjection,
  registeredFamilyProjections,
} from '../registry.ts';
import type { FlowModel, FlowProjection } from './model.ts';

export function registerFlowProjection<TOptions>(
  projection: FlowProjection<TOptions>,
  view = DEFAULT_VIEW,
): FlowProjection<TOptions> {
  return registerFamilyProjection(projection, view) as FlowProjection<TOptions>;
}

export function flowProjection(kind: string, view?: string): FlowProjection<unknown> | undefined {
  return familyProjection<FlowModel, unknown>('flow', kind, view) as FlowProjection<unknown> | undefined;
}

export function ensureFlowProjection(
  kind: string,
  view: string | undefined,
  onReady: () => void,
): FlowProjection<unknown> | 'loading' | 'unknown' {
  return ensureFamilyProjection<FlowModel, unknown>('flow', kind, view, onReady) as FlowProjection<unknown> | 'loading' | 'unknown';
}

export function registeredFlowKinds(): string[] {
  return [...new Set(registeredFamilyProjections('flow').map(entry => entry.kind))];
}
