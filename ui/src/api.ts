import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type {
  CountrySeries as GeneratedCountrySeries,
  CountrySeriesRequest,
  ReplayMetadata as GeneratedReplayMetadata,
} from './generated/analytics';

export type CountrySeries = GeneratedCountrySeries & { revision: string };
export type ReplayMetadata = GeneratedReplayMetadata & { revision: string };

export type {
  BranchMetadata,
  CountryMetadata,
  CountrySample,
  CountrySeriesPoint,
  CountrySeriesRequest,
  CountryValues,
} from './generated/analytics';

export const desktopAvailable = isTauri();

export interface ViewerState {
  path: string | null;
  revision: string | null;
  branch: string | null;
  hour: number | null;
}

export interface DesktopState {
  running: boolean;
  ready: boolean;
  visible: boolean;
  viewer: ViewerState;
  error: string | null;
}

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

export function chooseReplay(): Promise<string | null> {
  requireDesktop();
  return invoke<string | null>('choose_replay');
}

export function openReplay(path: string): Promise<void> {
  requireDesktop();
  return invoke<void>('open_replay', { path });
}

export function setMapView(view: MapView): Promise<void> {
  requireDesktop();
  return invoke<void>('set_map_view', { visible: view.visible, bounds: view.bounds });
}

export function seekMap(path: string, revision: string, branch: string, hour: number): Promise<void> {
  requireDesktop();
  return invoke<void>('seek_map', { path, revision, branch, hour });
}

let analyticsGeneration = 0;
async function analyticsRequest<T>(
  command: string,
  args: Record<string, unknown>,
  signal: AbortSignal,
): Promise<T> {
  requireDesktop();
  if (signal.aborted) throw new DOMException('Request cancelled', 'AbortError');
  const generation = ++analyticsGeneration;
  const requestId = crypto.randomUUID();
  const abort = () => {
    // A previous tab's cleanup may arrive after a new query has started. The
    // native worker cancels only this identity, leaving the new query intact.
    void invoke<void>('cancel_analytics', { requestId }).catch(() => {});
  };
  signal.addEventListener('abort', abort, { once: true });

  try {
    const result = await invoke<T>(command, { ...args, requestId });
    if (signal.aborted || generation !== analyticsGeneration) {
      throw new DOMException('Request cancelled', 'AbortError');
    }
    return result;
  } finally {
    signal.removeEventListener('abort', abort);
  }
}

export function getReplayMetadata(path: string, signal: AbortSignal): Promise<ReplayMetadata> {
  return analyticsRequest<ReplayMetadata>('analytics_metadata', { path }, signal);
}

export function getCountrySeries(
  path: string,
  request: CountrySeriesRequest,
  signal: AbortSignal,
): Promise<CountrySeries> {
  return analyticsRequest<CountrySeries>('analytics_series', { path, request }, signal);
}

export function requestCancelled(error: unknown): boolean {
  return (error instanceof DOMException && error.name === 'AbortError')
    || errorMessage(error) === 'Analytics request superseded';
}

export function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  return 'The request could not be completed. Please try again.';
}

export function recordingName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;
}
