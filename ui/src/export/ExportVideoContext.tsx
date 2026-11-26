import { createContext, useContext } from 'react';
import type { ExportJobSnapshot } from './videoExportApi';
import { IDLE_EXPORT_JOB } from './videoExportApi';

export interface MapVideoExportTarget {
  viewId?: string;
  label: string;
}

export interface ExportVideoController {
  openMapVideoExport: (target: MapVideoExportTarget) => void;
  job: ExportJobSnapshot;
}

export const ExportVideoContext = createContext<ExportVideoController>({
  openMapVideoExport: () => undefined,
  job: IDLE_EXPORT_JOB,
});

export function useExportVideo(): ExportVideoController {
  return useContext(ExportVideoContext);
}
