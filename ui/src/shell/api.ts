import { invoke, isTauri, transformCallback } from '@tauri-apps/api/core';
import type { CloseBatch, CloseRequests } from './closeGuard.ts';
import { lastInstallAttempt, watchedDocument } from './installAttempt';
import { listen } from '@tauri-apps/api/event';
import type { CameraMode, FlyAircraftStatus, ReplayMetadata as GeneratedReplayMetadata } from '../generated/dashboard';
import type { MapPlayback } from '../dashboard/mapPlayback';

export type ReplayMetadata = GeneratedReplayMetadata & { revision: string };

export type { BranchMetadata, CountryMetadata } from '../generated/dashboard';

export const desktopAvailable = isTauri();

export interface ViewerState {
  path: string | null;
  revision: string | null;
  branch: string | null;
  hour: number | null;
}

export interface DesktopState {
  mapDetails?: MapDetails | null;
  running: boolean;
  ready: boolean;
  visible: boolean;
  viewer: ViewerState;
  error: string | null;
  cameraMode: CameraMode;
  cameraOverlays: boolean;
  flyAircraft?: FlyAircraftStatus | null;
}

export interface MapOptions { country: string | null; countrySlot?: number | null; countryPinned?: boolean; fog: boolean; mapMode?: string | null; mapModeSlots?: string[]; pickerHidden?: boolean; observed?: number | null }
export interface ContentFolders { game: string; user: string; contentReady: boolean; warning?: string | null }
export interface MapDetails extends ContentFolders { error: string | null; path: string | null; branch: string | null; hour: number | null; messages: string[] }
export type MapAction = { kind: 'configure'; options: MapOptions } | { kind: 'inspect' } | { kind: 'unload'; path: string } | { kind: 'cameraMode'; mode: CameraMode; overlays: boolean } | { kind: 'flyAircraft'; aircraft: string | null } | { kind: 'refreshVisualMods' };
export function mapAction(action: MapAction): Promise<void> { requireDesktop(); return invoke('map_action', { action }); }
export interface SavedExport { message: string; warning: string | null }
export function exportMap(viewId?: string): Promise<SavedExport | null> { requireDesktop(); return invoke('export_map', { viewId: viewId ?? null }); }

export interface MapBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface MapView {
  visible: boolean;
  bounds: MapBounds;
}

function requireDesktop(): void {
  if (!desktopAvailable) {
    throw new Error('Open BattlePlan in the desktop app to use your recordings and the map.');
  }
}

export function getDesktopState(): Promise<DesktopState> {
  requireDesktop();
  return invoke<DesktopState>('desktop_state');
}

export function onViewerState(callback: (state: DesktopState) => void) {
  requireDesktop();
  return listen<DesktopState>('viewer-state', (event) => callback(event.payload));
}

export function openReplay(path: string): Promise<void> {
  requireDesktop();
  return invoke<void>('open_replay', { path });
}

export function setMapView(view: MapView): Promise<void> {
  requireDesktop();
  return invoke<void>('set_map_view', { visible: view.visible, bounds: view.bounds });
}

export function setMapHoles(holes: MapBounds[]): Promise<void> {
  requireDesktop();
  return invoke<void>('set_map_holes', { holes });
}

export function selectNativeView(dashboard: boolean): Promise<void> {
  if (!desktopAvailable) return Promise.resolve();
  return invoke<void>('select_native_view', { dashboard });
}

export function seekMap(path: string, revision: string, branch: string, hour: number, playback?: MapPlayback): Promise<void> {
  requireDesktop();
  return invoke<void>('seek_map', { path, revision, branch, hour, playback });
}

let closeWatches = 0;

export function closeRequests(onBatch: (batch: CloseBatch) => boolean): CloseRequests {
  if (!desktopAvailable) return { watch: () => Promise.resolve(), refresh: (_givingUpRestarts: boolean) => Promise.reject('BattlePlan’s desktop host isn’t here, so the page can’t ask it what stands.') };
  const seq = ++closeWatches;
  const callback = transformCallback<CloseBatch>(batch => onBatch(batch) === true);
  let watched: number | null = null;
  return {
    watch: async () => {
      const document = await invoke<number>('close_watch_document');
      watched = document;
      watchedDocument(document);
      await invoke<void>('watch_close_requests', { document, seq, callback });
    },
    refresh: async (givingUpRestarts: boolean) => {
      if (watched === null) throw 'This page hasn’t watched for BattlePlan closing yet, so it can’t ask again what stands.';
      return await invoke<number>('refresh_close_requests', { document: watched, seq, attempt: givingUpRestarts ? lastInstallAttempt() : 0 });
    },
  };
}

export function answerCloseQuestion(id: number, stays: boolean, basis: readonly number[]): Promise<boolean> {
  requireDesktop();
  return invoke<boolean>('answer_close_question', { id, stays, basis });
}

export function leaveApp(): Promise<void> {
  requireDesktop();
  return invoke<void>('leave_app');
}

export { errorMessage } from './errors';

export function recordingName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;
}
