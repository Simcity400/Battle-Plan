/** The seam to the game-window renderer's division designer. The renderer
 * registers itself here once it can draw a recorded template; until then a
 * template row has nothing to open. The widget hands over an identity, never a
 * tag, name or summary, and shows the renderer's own refusal when it cannot draw. */
import type { ReactNode } from 'react';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import type { DesignerKind } from '../../generated/dashboard';

export interface DesignerRequest {
  client: DashboardClient;
  source: DashboardSource;
  branchId: string;
  /** The exact hour the row was read at. */
  hour: number;
  /** The country holding the roster, at that hour. */
  tag: string;
  /** The template's full native handle. */
  template: string;
  designer: DesignerKind;
  /** Receives the window's own diagnostics, which the widget reports beside its own. */
  onDiagnostics?: (lines: string[]) => void;
}

/** Draws the designer for a request, with its own loading and refusal states. */
export type DesignerSnapshot = (request: DesignerRequest, active: boolean) => ReactNode;

let snapshot: DesignerSnapshot | null = null;
const listeners = new Set<() => void>();

/** The renderer lane calls this once its designer snapshot exists. */
export function registerDesignerSnapshot(draw: DesignerSnapshot): void {
  snapshot = draw;
  for (const listener of listeners) listener();
}
export function designerSnapshot(): DesignerSnapshot | null { return snapshot; }
export function subscribeDesignerSnapshot(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}
