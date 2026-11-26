import { createContext, useContext } from 'react';
import type { LaneMotion } from '../families/laneRuntime';

export interface ExportWidgetStatus {
  readonly widgetId: string;
  readonly pipe: number | null;
  readonly mode: string;
  readonly target: number | null;
  readonly binding: number;
  readonly installedBinding: number;
  readonly installedId: number | null;
  readonly expired: boolean;
  readonly pendingIdentities: boolean;
  readonly pendingArt: boolean;
}

export interface ExportSurfaceContextValue {
  readonly active: boolean;
  readonly motion?: LaneMotion;
  readonly reportWidget?: (key: string, status: ExportWidgetStatus | null) => void;
}

const DEFAULT_EXPORT_SURFACE: ExportSurfaceContextValue = {
  active: false,
};

export const ExportSurfaceContext = createContext<ExportSurfaceContextValue>(DEFAULT_EXPORT_SURFACE);

export function useExportSurface(): ExportSurfaceContextValue {
  try {
    return useContext(ExportSurfaceContext);
  } catch {
    return DEFAULT_EXPORT_SURFACE;
  }
}
