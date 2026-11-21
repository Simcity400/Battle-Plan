import type { ReactNode } from 'react';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import type { DesignerKind } from '../../generated/dashboard';

export interface DesignerRequest {
  client: DashboardClient;
  source: DashboardSource;
  branchId: string;
  hour: number;
  tag: string;
  template: string;
  designer: DesignerKind;
  onDiagnostics?: (lines: string[]) => void;
}

export type DesignerSnapshot = (request: DesignerRequest, active: boolean) => ReactNode;

let snapshot: DesignerSnapshot | null = null;
const listeners = new Set<() => void>();

export function registerDesignerSnapshot(draw: DesignerSnapshot): void {
  snapshot = draw;
  for (const listener of listeners) listener();
}
export function designerSnapshot(): DesignerSnapshot | null { return snapshot; }
export function subscribeDesignerSnapshot(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}
