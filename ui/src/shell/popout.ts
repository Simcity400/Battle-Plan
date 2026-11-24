import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { desktopAvailable } from './api';
import type { DesktopState, MapOptions } from './api';
import type { CameraMode } from '../generated/dashboard';
import type { MapPlayback } from '../dashboard/mapPlayback';

export interface PopOutState { generation: number; sequence: number; viewer: number; open: boolean; status: DesktopState | null; error?: string | null }
export interface PopOutStatus { generation: number; sequence: number; viewer: number; status: DesktopState }
export interface PopOutError { generation: number; sequence: number; error: string }

export const CLOSED: PopOutState = { generation: 0, sequence: 0, viewer: 0, open: false, status: null, error: null };

export const openPopout = () => invoke<void>('popout_open');
export const closePopout = (generation: number) => invoke<void>('popout_close', { generation });
export const restartPopout = (generation: number) => invoke<void>('popout_restart', { generation });
export const nextPopoutMonitor = (generation: number) => invoke<void>('popout_next_monitor', { generation });
export const popoutUnloaded = (path: string, generation: number) => invoke<void>('popout_unloaded', { path, generation });
export const seekPopout = (generation: number, path: string, revision: string, branch: string, hour: number, playback?: MapPlayback) =>
  invoke<void>('popout_seek', { generation, path, revision, branch, hour, playback });
export const popoutOptions = (options: MapOptions): MapOptions => ({ ...options, pickerHidden: true });
export const configurePopout = (generation: number, options: MapOptions) =>
  invoke<void>('popout_configure', { generation, options: popoutOptions(options) });
export const setPopoutCameraMode = (generation: number, mode: CameraMode, overlays: boolean) =>
  invoke<void>('popout_camera_mode', { generation, mode, overlays });
export const setPopoutFlyAircraft = (generation: number, aircraft: string | null) => invoke<void>('popout_fly_aircraft', { generation, aircraft });

export function nextPopoutState(previous: PopOutState, payload: PopOutState | PopOutStatus): PopOutState {
  if (payload.sequence <= previous.sequence) return previous;
  if ('open' in payload) return { ...payload, error: payload.error ?? null };
  return previous.open && payload.generation === previous.generation ? { ...previous, sequence: payload.sequence, viewer: payload.viewer, status: payload.status } : previous;
}

export function popoutFailed(previous: PopOutState, payload: PopOutError): PopOutState {
  if (payload.sequence <= previous.sequence || !previous.open || payload.generation !== previous.generation) return previous;
  return { ...previous, sequence: payload.sequence, error: payload.error };
}

export function popoutProblem(state: PopOutState): string | null {
  if (!state.open) return null;
  if (state.error) return state.error;
  if (state.status && !state.status.running && state.status.error) return `The pop-out map stopped: ${state.status.error}`;
  return state.status?.error ?? null;
}

export const popoutClockKey = (state: PopOutState) => state.open && state.status?.running ? `${state.generation}:${state.viewer}` : null;

export const popoutConfigKey = (state: PopOutState) => state.open && state.status?.ready ? `${state.generation}:${state.viewer}` : null;

export const popoutStopped = (state: PopOutState) => state.open && !!state.status && !state.status.running;

type Listen = <T>(event: string, handler: (event: { payload: T }) => void) => Promise<() => void>;
type Invoke = (command: string) => Promise<unknown>;

export function subscribePopout(listen: Listen, invoke: Invoke, apply: (update: (previous: PopOutState) => PopOutState) => void): () => void {
  let current = true;
  const status = listen<PopOutState | PopOutStatus>('popout-state', event => { if (current) apply(previous => nextPopoutState(previous, event.payload)); });
  const failure = listen<PopOutError>('popout-error', event => { if (current) apply(previous => popoutFailed(previous, event.payload)); });
  void Promise.all([status, failure]).then(() => current ? invoke('popout_publish') : undefined).catch(() => {});
  return () => {
    current = false;
    void status.then(stop => stop(), () => {});
    void failure.then(stop => stop(), () => {});
  };
}

export function usePopout(): PopOutState {
  const [state, setState] = useState<PopOutState>(CLOSED);
  useEffect(() => desktopAvailable ? subscribePopout(listen, invoke, setState) : undefined, []);
  return state;
}
