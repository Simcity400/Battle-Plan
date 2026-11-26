import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export type BackendChoice = 'auto' | 'mediaFoundation' | 'ffmpeg';
export type Quality = 'standard' | 'high' | 'maximum';

export interface BackendAvailability {
  mediaFoundation: boolean;
  mediaFoundationDetail: string;
  ffmpeg: boolean;
  ffmpegPath: string | null;
  ffmpegDetail: string;
}

export interface ExportSettings {
  backend: BackendChoice;
  ffmpegPath: string | null;
}

export interface CropRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export type SavedPose =
  | { kind: 'map' }
  | { kind: 'free'; yaw: number; pitch: number; roll: number; fovY: number; near: number; far: number };

export interface SavedPlacement {
  center: [number, number];
  height: number;
  pose: SavedPose;
}

export interface RecordedStretch {
  first: number;
  last: number;
  join: 'start' | 'load' | 'break';
}

export interface UnheldGap {
  first: number;
  last: number;
  reason: string;
}

export interface MapExportSnapshot {
  path: string;
  revision: string | null;
  branch: number;
  currentHour: number;
  placement: SavedPlacement;
  viewport: [number, number];
  uiScale: number;
  mapMode: string;
  perspective: string | null;
  perspectiveSlot?: number | null;
  fog: boolean;
  dayNight: boolean;
  alliedPlans: boolean;
  counterColorsCountry: boolean;
  natoCounters: boolean;
  audioMuted: boolean;
  mapSettings?: unknown;
  maxTextureDimension: number;
  stretches: RecordedStretch[];
  unheld: UnheldGap[];
  firstHour: number;
  lastHour: number;
}

export interface MapVideoRequest {
  path: string;
  revision: string | null;
  branch: number;
  fromHour: number;
  toHour: number;
  fps: number;
  hoursPerSecond: number;
  skipGaps: boolean;
  cutCardSeconds: number;
  width: number;
  height: number;
  quality: Quality;
  backend: BackendChoice;
  ffmpegPath: string | null;
  placement: SavedPlacement | null;
  sourceViewport: [number, number] | null;
  crop: CropRect | null;
  uiScale: number | null;
  mapMode: string | null;
  perspective: string | null;
  perspectiveSlot?: number | null;
  fog: boolean;
  dayNight: boolean | null;
  alliedPlans: boolean | null;
  counterColorsCountry: boolean | null;
  natoCounters: boolean | null;
  mapSettings?: unknown;
  dateStamp: boolean;
  audioEnabled: boolean;
  allowUnverified: boolean;
}

export interface PartialExportInfo {
  throughHour: number;
  frames: number;
  durationSeconds: number;
}

export interface ExportJobSnapshot {
  id: number;
  state: 'idle' | 'running' | 'completed' | 'cancelled' | 'failed';
  targetLabel: string;
  destination: string | null;
  phase: string;
  frame: number;
  totalFrames: number;
  campaignHour: number | null;
  elapsedMs: number;
  etaMs: number | null;
  backend: string | null;
  warnings: string[];
  error: string | null;
  partial: PartialExportInfo | null;
}

export const IDLE_EXPORT_JOB: ExportJobSnapshot = {
  id: 0,
  state: 'idle',
  targetLabel: '',
  destination: null,
  phase: '',
  frame: 0,
  totalFrames: 0,
  campaignHour: null,
  elapsedMs: 0,
  etaMs: null,
  backend: null,
  warnings: [],
  error: null,
  partial: null,
};

function requireDesktop(): void {
  if (!isTauri()) {
    throw new Error('Video export is available in the BattlePlan desktop app.');
  }
}

export function getExportSettings(): Promise<ExportSettings> {
  requireDesktop();
  return invoke<ExportSettings>('export_settings');
}

export function setExportSettings(settings: ExportSettings): Promise<ExportSettings> {
  requireDesktop();
  return invoke<ExportSettings>('set_export_settings', { settings });
}

export function probeExportBackends(ffmpegPath?: string | null): Promise<BackendAvailability> {
  requireDesktop();
  return invoke<BackendAvailability>('export_backends', { ffmpegPath: ffmpegPath ?? null });
}

export function chooseFfmpegBinary(): Promise<string | null> {
  requireDesktop();
  return invoke<string | null>('choose_ffmpeg_binary');
}

export function getVideoExportState(): Promise<ExportJobSnapshot> {
  if (!isTauri()) return Promise.resolve(IDLE_EXPORT_JOB);
  return invoke<ExportJobSnapshot>('video_export_state');
}

export function getMapExportSnapshot(viewId?: string): Promise<MapExportSnapshot> {
  requireDesktop();
  return invoke<MapExportSnapshot>('map_export_snapshot', { viewId: viewId ?? null });
}

export function chooseVideoExportPath(defaultName: string): Promise<string | null> {
  requireDesktop();
  return invoke<string | null>('choose_video_export_path', { defaultName });
}

export async function previewMapVideo(request: MapVideoRequest, previewHour: number): Promise<Uint8Array<ArrayBuffer>> {
  requireDesktop();
  const data = await invoke<ArrayBuffer | Uint8Array | number[]>('preview_map_video', { request, previewHour });
  if (data instanceof ArrayBuffer) return new Uint8Array(data);
  if (data instanceof Uint8Array) {
    const copy = new Uint8Array(new ArrayBuffer(data.byteLength));
    copy.set(data);
    return copy;
  }
  const copy = new Uint8Array(new ArrayBuffer(data.length));
  copy.set(data);
  return copy;
}

export interface SurfaceTrackCoverage {
  stretches: RecordedStretch[];
  unheld: UnheldGap[];
}

export interface EmbeddedMapSpec {
  widgetId: string;
  path: string;
  revision: string | null;
  branch: number;
  axisOffset: number;
  pinnedHour: number | null;
  placement: SavedPlacement | null;
  sourceViewport: [number, number] | null;
  uiScale: number | null;
  mapMode: string | null;
  perspective: string | null;
  fog: boolean;
  dayNight: boolean | null;
  alliedPlans: boolean | null;
  counterColorsCountry: boolean | null;
  natoCounters: boolean | null;
}

export interface SurfaceVideoRequest {
  kind: 'widget' | 'page';
  fromHour: number;
  toHour: number;
  fps: number;
  hoursPerSecond: number;
  skipGaps: boolean;
  cutCardSeconds: number;
  width: number;
  height: number;
  quality: Quality;
  backend: BackendChoice;
  ffmpegPath: string | null;
  framing: 'full' | 'visible';
  reflowForPortrait: boolean;
  sourceWidth: number;
  sourceHeight: number;
  scrollTop: number;
  dateStamp: boolean;
  followPinned: boolean;
  audioMapWidgetId: string | null;
  tracks: SurfaceTrackCoverage[];
  embeddedMaps: EmbeddedMapSpec[];
  payload: unknown;
}

export interface ExportMapRect {
  widgetId: string;
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface PageAckPayload {
  seq: number;
  kind: 'boot' | 'mount' | 'metrics' | 'step';
  dirty: boolean;
  contentWidth: number | null;
  contentHeight: number | null;
  minTextPx: number | null;
  mapRects: ExportMapRect[];
  error: string | null;
}

export interface SurfacePreviewResult {
  pngBase64: string;
  minTextPx: number | null;
  contentWidth: number | null;
  contentHeight: number | null;
  scale: number;
}

export function getSurfaceExportTimelines(
  tracks: { path: string; branch: number; axisOffset: number }[],
): Promise<SurfaceTrackCoverage[]> {
  requireDesktop();
  return invoke<SurfaceTrackCoverage[]>('surface_export_timelines', { tracks });
}

export function previewSurfaceVideo(
  request: SurfaceVideoRequest,
  previewHour: number,
): Promise<SurfacePreviewResult> {
  requireDesktop();
  return invoke<SurfacePreviewResult>('preview_surface_video', { request, previewHour });
}

export function startSurfaceVideoExport(
  request: SurfaceVideoRequest,
  destination: string,
  targetLabel: string,
): Promise<ExportJobSnapshot> {
  requireDesktop();
  return invoke<ExportJobSnapshot>('start_surface_video_export', {
    args: { request, destination, targetLabel },
  });
}

export function sendVideoExportPageAck(payload: PageAckPayload): Promise<void> {
  return invoke<void>('video_export_page_ack', { payload });
}

export function startMapVideoExport(
  request: MapVideoRequest,
  destination: string,
  targetLabel: string,
): Promise<ExportJobSnapshot> {
  requireDesktop();
  return invoke<ExportJobSnapshot>('start_map_video_export', {
    args: { request, destination, targetLabel },
  });
}

export function cancelVideoExport(): Promise<ExportJobSnapshot> {
  requireDesktop();
  return invoke<ExportJobSnapshot>('cancel_video_export');
}

export function savePartialVideoExport(jobId: number): Promise<string> {
  requireDesktop();
  return invoke<string>('save_partial_video_export', { jobId });
}

export function dismissVideoExport(): Promise<ExportJobSnapshot> {
  requireDesktop();
  return invoke<ExportJobSnapshot>('dismiss_video_export');
}

export function revealExportFile(path: string): Promise<void> {
  requireDesktop();
  return invoke<void>('reveal_export_file', { path });
}

export function onVideoExportProgress(callback: (job: ExportJobSnapshot) => void): Promise<() => void> {
  if (!isTauri()) return Promise.resolve(() => undefined);
  return listen<ExportJobSnapshot>('video-export-progress', event => callback(event.payload));
}
