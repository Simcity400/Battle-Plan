// The updater both apps share (crates/app-update): one version, one installer, one channel.
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { UnlistenFn } from '@tauri-apps/api/event';

export type UpdateStatus = 'disabled' | 'idle' | 'checking' | 'up-to-date' | 'available' | 'downloading' | 'downloaded' | 'error';
export type UpdateChannel = 'stable' | 'nightly';
export type UpdateBuild = 'stable' | 'nightly' | 'preview' | 'development';

export interface UpdateState {
  enabled: boolean;
  status: UpdateStatus;
  channel: UpdateChannel;
  build: UpdateBuild;
  currentVersion: string;
  availableVersion: string | null;
  downloadedVersion: string | null;
  /** The offered release's publication time, as its feed gives it. */
  releasedAt: string | null;
  /** The offered release's changes, one per line. */
  notes: string[];
  downloadPercent: number | null;
  /** Milliseconds since 1970 of the last finished check. */
  checkedAt: number | null;
  /** Why the last action failed, or why updates are off. */
  message: string | null;
  errorContext: 'check' | 'download' | 'install' | null;
  /** Why the downloaded update cannot be installed now: the recorder is busy. */
  installBlocked: string | null;
}

export const updatesAvailable = isTauri();

export const updateState = () => invoke<UpdateState>('app_update_state');
export const checkForUpdate = () => invoke<UpdateState>('app_update_check');
export const downloadUpdate = () => invoke<UpdateState>('app_update_download');
/** Resolves only when the installer did not start; once it starts, both apps close. */
export const installUpdate = () => invoke<void>('app_update_install');
export const setUpdateChannel = (channel: UpdateChannel) => invoke<UpdateState>('app_update_set_channel', { channel });
export const openRelease = (version: string) => invoke<void>('app_update_open_release', { version });
export const onUpdateState = (handler: (state: UpdateState) => void): Promise<UnlistenFn> =>
  listen<UpdateState>('app-update-state', event => handler(event.payload));
