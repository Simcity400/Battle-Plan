import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { CameraMode, FlyAircraftStatus, ReplayMetadata as GeneratedReplayMetadata } from '../generated/dashboard';

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
  /** The full map's camera mode, as the viewer last reported it. */
  cameraMode: CameraMode;
  /** Whether flying and walking draw the map's interface, as the viewer last reported it. */
  cameraOverlays: boolean;
  /** The aircraft the full map was told to fly, as it last answered; none until it has been told. */
  flyAircraft?: FlyAircraftStatus | null;
}

/** `mapMode` is a picker mode's token or `scripted:<name>`; null is the political map. */
/** `pickerHidden` hides the native picker (the map-only window); its keys still work. */
export interface MapOptions { country: string | null; fog: boolean; mapMode?: string | null; mapModeSlots?: string[]; pickerHidden?: boolean }
export interface ContentFolders { game: string; user: string; contentReady: boolean }
export interface MapDetails extends ContentFolders { error: string | null; path: string | null; branch: string | null; hour: number | null; messages: string[] }
export type MapAction = { kind: 'configure'; options: MapOptions } | { kind: 'inspect' } | { kind: 'unload'; path: string } | { kind: 'cameraMode'; mode: CameraMode; overlays: boolean } | { kind: 'flyAircraft'; aircraft: string | null } | { kind: 'refreshVisualMods' };
export function mapAction(action: MapAction): Promise<void> { requireDesktop(); return invoke('map_action', { action }); }
export function exportMap(viewId?: string): Promise<void> { requireDesktop(); return invoke('export_map', { viewId: viewId ?? null }); }

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
    throw new Error('Open Battle Plan in the desktop app to use local recordings and the native map.');
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

export function selectNativeView(dashboard: boolean): Promise<void> {
  if (!desktopAvailable) return Promise.resolve();
  return invoke<void>('select_native_view', { dashboard });
}

export function seekMap(path: string, revision: string, branch: string, hour: number, playback?: { playing: boolean; seekSequence: number }): Promise<void> {
  requireDesktop();
  return invoke<void>('seek_map', { path, revision, branch, hour, playback });
}

export { errorMessage } from './errors';

export function recordingName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;
}
