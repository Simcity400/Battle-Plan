import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { desktopAvailable } from './api';
import type { DesktopState, MapOptions } from './api';
import type { CameraMode } from '../generated/dashboard';

/** The pop-out map window: the campaign map alone, driven by this app. Each window opened is a
 * new `generation`, and every event names the one it belongs to; each viewer started in it, first
 * or restarted, is a new `viewer`. Every event also carries the host's `sequence`, which increases
 * with each event it publishes, so the state shown is always the newest the host reported.
 * `error` is a failure its window reported and has not recovered from, such as a resize. */
export interface PopOutState { generation: number; sequence: number; viewer: number; open: boolean; status: DesktopState | null; error?: string | null }
/** A pop-out viewer's own status, naming its window's generation and the viewer. */
export interface PopOutStatus { generation: number; sequence: number; viewer: number; status: DesktopState }
/** A failure the pop-out's window or viewer could not report itself, naming its generation. */
export interface PopOutError { generation: number; sequence: number; error: string }

export const CLOSED: PopOutState = { generation: 0, sequence: 0, viewer: 0, open: false, status: null, error: null };

/** Opens the pop-out; its state arrives as a `popout-state` event. */
export const openPopout = () => invoke<void>('popout_open');
export const closePopout = (generation: number) => invoke<void>('popout_close', { generation });
export const restartPopout = (generation: number) => invoke<void>('popout_restart', { generation });
/** Asks the host to publish the pop-out's whole state as a `popout-state` event. */
export const publishPopout = () => invoke<void>('popout_publish');
/** Moves pop-out window `generation`, the one shown when asked, to the next monitor. */
export const nextPopoutMonitor = (generation: number) => invoke<void>('popout_next_monitor', { generation });
/** A recording the user unloaded while pop-out window `generation` was shown; that window closes
 * if it shows the recording. Reconnecting does not call this. */
export const popoutUnloaded = (path: string, generation: number) => invoke<void>('popout_unloaded', { path, generation });
export const seekPopout = (generation: number, path: string, revision: string, branch: string, hour: number, playback?: { playing: boolean; seekSequence: number }) =>
  invoke<void>('popout_seek', { generation, path, revision, branch, hour, playback });
export const configurePopout = (generation: number, options: MapOptions) => invoke<void>('popout_configure', { generation, options });
/** Asks pop-out window `generation`'s map to move its camera as `mode`, showing the map's interface in a free
 * mode when `overlays`; its status reports the mode and choice it takes. */
export const setPopoutCameraMode = (generation: number, mode: CameraMode, overlays: boolean) =>
  invoke<void>('popout_camera_mode', { generation, mode, overlays });

/** The `popout-state` event carries two payloads: the whole state from opening and closing, and
 * the viewer's own status in between. An event no newer than the state shown changes nothing; a
 * newer whole state replaces it, and a newer status counts only for the open window it names. */
export function nextPopoutState(previous: PopOutState, payload: PopOutState | PopOutStatus): PopOutState {
  if (payload.sequence <= previous.sequence) return previous;
  // A whole state carries the window's unrecovered error, so it never erases one.
  if ('open' in payload) return { ...payload, error: payload.error ?? null };
  return previous.open && payload.generation === previous.generation ? { ...previous, sequence: payload.sequence, viewer: payload.viewer, status: payload.status } : previous;
}

/** The pop-out with a failure its window or viewer reported, when it is newer than the state
 * shown and names the open window. */
export function popoutFailed(previous: PopOutState, payload: PopOutError): PopOutState {
  if (payload.sequence <= previous.sequence || !previous.open || payload.generation !== previous.generation) return previous;
  return { ...previous, sequence: payload.sequence, error: payload.error };
}

/** Why the pop-out shows nothing, or null while it is fine. */
export function popoutProblem(state: PopOutState): string | null {
  if (!state.open) return null;
  if (state.error) return state.error;
  if (state.status && !state.status.running && state.status.error) return `The pop-out map stopped: ${state.status.error}`;
  return state.status?.error ?? null;
}

/** The viewer the clock drives, as `generation:viewer`, while its window is open and it runs,
 * else null. Each viewer, in a new window or restarted in the same one, has its own key, so it
 * gets its own subscription, which sends it the current hour and playback; a seek a stopped viewer
 * refused is not lost. */
export const popoutClockKey = (state: PopOutState) => state.open && state.status?.running ? `${state.generation}:${state.viewer}` : null;

/** The viewer that takes the map options, as `generation:viewer`, while its window is open and it
 * is ready, else null. Each viewer, new window or restart, is sent the options again. */
export const popoutConfigKey = (state: PopOutState) => state.open && state.status?.ready ? `${state.generation}:${state.viewer}` : null;

/** Whether the pop-out's viewer exited and can be started again. */
export const popoutStopped = (state: PopOutState) => state.open && !!state.status && !state.status.running;

type Listen = <T>(event: string, handler: (event: { payload: T }) => void) => Promise<() => void>;
type Invoke = (command: string) => Promise<unknown>;

/** Keeps the pop-out's state current from the host's events. Both listeners are registered before
 * the host is asked to publish the whole state, so no event after that answer is missed and the
 * answer itself arrives as an event. Returns a stop function, which also works before the
 * listeners finish registering. */
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

/** The pop-out's state, kept current from the host's events. */
export function usePopout(): PopOutState {
  const [state, setState] = useState<PopOutState>(CLOSED);
  useEffect(() => desktopAvailable ? subscribePopout(listen, invoke, setState) : undefined, []);
  return state;
}
