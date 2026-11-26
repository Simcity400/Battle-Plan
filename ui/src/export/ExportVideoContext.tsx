import { createContext, useContext } from 'react';
import type { DashboardTemplate, WidgetSpec } from '../dashboard/model';
import type { CountryChoice } from '../dashboard/mapPicker';
import type { ExportJobSnapshot, RecordedStretch, UnheldGap } from './videoExportApi';
import { IDLE_EXPORT_JOB } from './videoExportApi';

export interface MapVideoExportTarget {
  kind: 'map';
  viewId?: string;
  label: string;
}

export interface SurfaceExportTrackSnapshot {
  id: string;
  roleId: string;
  path: string;
  revision: string;
  branchId: string;
  axisOffset: number;
  firstHour: number;
  lastHour: number;
  stretches: RecordedStretch[];
  unheld: UnheldGap[];
}

export interface SurfaceExportMapWidget {
  widgetId: string;
  title: string;
  trackId: string;
  path: string;
  revision: string | null;
  branch: number;
  axisOffset: number;
  timeMode: 'follow' | 'pin' | 'campaign' | 'range';
  fixedHour: number | null;
  fog: boolean;
  perspective: string | null;
  perspectiveSlot: number | null;
}

export interface SurfaceExportContextSnapshot {
  campaigns: { id: string; path: string; label: string; revision: string }[];
  selectedCountries: Record<string, CountryChoice | null>;
  selectedBranches: Record<string, string>;
  observed: Record<string, number>;
  defaultRoleId: string | null;
  referenceOffset: number;
  currentHour: number;
  firstHour: number;
  lastHour: number;
  sourceWidth: number;
  sourceHeight: number;
  scrollTop: number;
  tracks: SurfaceExportTrackSnapshot[];
  followTrackIds: string[];
  pinnedTrackIds: string[];
  fallbackTrackIds: string[];
  mapWidgets: SurfaceExportMapWidget[];
  hasPinnedWidgets: boolean;
  hasPagedRowsWidgets: boolean;
}

export interface WidgetVideoExportTarget {
  kind: 'widget';
  widget: WidgetSpec;
  template: DashboardTemplate;
  label: string;
  context: SurfaceExportContextSnapshot;
}

export interface PageVideoExportTarget {
  kind: 'page';
  pageId: string;
  template: DashboardTemplate;
  label: string;
  context: SurfaceExportContextSnapshot;
}

export type ExportVideoTarget =
  | MapVideoExportTarget
  | WidgetVideoExportTarget
  | PageVideoExportTarget;

export interface ExportVideoController {
  openMapVideoExport: (target: { viewId?: string; label: string }) => void;
  openWidgetVideoExport: (widget: WidgetSpec, template: DashboardTemplate) => void;
  openPageVideoExport: (pageId: string, label: string) => void;
  registerSurfaceSnapshot: (
    provider: {
      forWidget: (widget: WidgetSpec, template: DashboardTemplate) => WidgetVideoExportTarget | null;
      forPage: (pageId: string, label: string) => PageVideoExportTarget | null;
    } | null,
  ) => void;
  job: ExportJobSnapshot;
}

const DEFAULT_EXPORT_VIDEO_CONTROLLER: ExportVideoController = {
  openMapVideoExport: () => undefined,
  openWidgetVideoExport: () => undefined,
  openPageVideoExport: () => undefined,
  registerSurfaceSnapshot: () => undefined,
  job: IDLE_EXPORT_JOB,
};

export const ExportVideoContext = createContext<ExportVideoController>(DEFAULT_EXPORT_VIDEO_CONTROLLER);

export function useExportVideo(): ExportVideoController {
  try {
    return useContext(ExportVideoContext);
  } catch {
    return DEFAULT_EXPORT_VIDEO_CONTROLLER;
  }
}
